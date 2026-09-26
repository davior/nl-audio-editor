// One project: lanes, transport, monitor, view, stack, log, clone and bundle.
// Everything here is display and control; the core does the audio work.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Player } from "../audio/player";
import { TimeMap } from "../audio/timemap";
import { analyseInBackground, core } from "../core/client";
import type { EditMap, ProjectSummary, ViewState, Which } from "../core/types";
import { debug } from "../debug";
import { sync } from "../storage/sync";
import { IntegrityBadge } from "./IntegrityBadge";
import { Spectrogram, TimeAxis, Waveform } from "./Lanes";
import { LogViewer, type LogFailure } from "./LogViewer";
import { Console } from "./Console";
import { StackPanel, type StackActions } from "./StackPanel";
import { useDescriptors } from "./StepEditor";
import { DB_RANGES, fit, follow, restoreView, scroll, showRange, zoom } from "./viewState";

export interface EditorProps {
  summary: ProjectSummary;
  /** Open without a library copy: nothing is persisted. */
  detached: boolean;
  notice: string | null;
  onChanged: (s: ProjectSummary) => void;
  onOpenClone: (child: ProjectSummary) => void;
  onClose: () => void;
  onError: (e: unknown) => void;
  /** Set by the editor: saves a pending view change now (called before switching projects). */
  flushRef: React.MutableRefObject<(() => Promise<void>) | null>;
}

type Monitor = { which: Which; label: string; title: string; testId: string };

const MONITORS: Monitor[] = [
  { which: "source", label: "Original", title: "The recording as imported, untouched", testId: "monitor-source" },
  { which: "stack", label: "Processed", title: "The recording through the stack", testId: "monitor-stack" },
  {
    which: "residual",
    label: "Residual",
    title: "What the stack removed, at the level it would have had",
    testId: "monitor-residual",
  },
];

/** With a step selected: that step on its own, over the whole recording. */
function stepMonitors(id: string, op: string): Monitor[] {
  return [
    {
      which: `step:${id}:before`,
      label: "Before this step",
      title: `The recording through the steps below ${op}`,
      testId: "monitor-step-before",
    },
    { which: `step:${id}:after`, label: "After it", title: `The same, with ${op}`, testId: "monitor-step-after" },
    { which: `step:${id}:removed`, label: "What it removed", title: `What ${op} took out: before − after`, testId: "monitor-step-removed" },
  ];
}

const isTimeEdit = (op: string) => op === "remove_time" || op === "insert_silence";

type ExportFormat = "f32" | "pcm24" | "pcm16";

function fmtTime(t: number): string {
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(3).padStart(6, "0")}`;
}

function download(bytes: Uint8Array, name: string, type = "application/zip") {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name.replace(/[\\/:*?"<>|]/g, "_");
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

type Capture = {
  requested?: Record<string, unknown>;
  applied?: Record<string, unknown>;
  device_label?: string;
  context_sample_rate?: number;
};

export function Editor({ summary, detached, notice, onChanged, onOpenClone, onClose, onError, flushRef }: EditorProps) {
  const id = summary.id;
  const m = summary.manifest;
  const info = m.source_info;
  const duration = info.duration_s;
  const sampleRate = info.sample_rate;
  const readOnly = summary.readOnly;

  const [view, setView] = useState<ViewState>(() => restoreView(m.view, duration, sampleRate));
  const [events, setEvents] = useState<Record<string, unknown>[]>([]);
  const [playhead, setPlayhead] = useState(0);
  const [playing, setPlaying] = useState(false);
  // While the console's microphone is open, nothing plays, so no recording reaches the recogniser.
  const [dictating, setDictating] = useState(false);
  const dictatingRef = useRef(false);
  const [loop, setLoop] = useState(false);
  const [width, setWidth] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [renderHash, setRenderHash] = useState<{ which: Which; hash: string } | null>(null);
  // The step selected in the stack: its values can be changed, and it can be heard on its own.
  const [selected, setSelected] = useState<string | null>(null);
  const descriptors = useDescriptors(onError);
  const [exportFormat, setExportFormat] = useState<ExportFormat>("f32");
  // Where the stack's time edits put the original in the output.
  const [editMap, setEditMap] = useState<EditMap | null>(null);
  // Requests made with buttons go through the console, as if typed.
  const [consoleRequest, setConsoleRequest] = useState<{ words: string; n: number } | null>(null);
  const [insertLength, setInsertLength] = useState(1);
  // Analysis runs after the lanes appear, in a worker of its own.
  const [analysis, setAnalysis] = useState<"running" | "done" | "failed" | "not needed">(
    summary.needsAnalysis && !readOnly ? "running" : "not needed",
  );
  const lanes = useRef<HTMLDivElement>(null);
  const player = useRef<Player>(new Player());
  const playFrom = useRef(0);
  const viewRef = useRef(view);
  viewRef.current = view;

  // Lane width follows the window.
  useEffect(() => {
    const el = lanes.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(200, Math.floor(el.clientWidth))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    core.events(id).then(setEvents, onError);
  }, [id, summary, onError]);

  // The lanes come first: the analysis and the playback copy of the audio
  // wait until the first view has been drawn (or a few seconds, whichever is first).
  const [lanesReady, setLanesReady] = useState(false);
  useEffect(() => {
    const h = setTimeout(() => setLanesReady(true), 3000);
    return () => clearTimeout(h);
  }, []);

  const latest = useRef({ onChanged, onError, detached });
  latest.current = { onChanged, onError, detached };
  useEffect(() => {
    debug("analysis", analysis);
    if (analysis !== "running" || !lanesReady) return;
    let live = true;
    (async () => {
      const started = performance.now();
      const pcm = await core.pcm(id, "source");
      const { features, renderHash } = await analyseInBackground(pcm);
      if (!live) return;
      const s = await core.recordAnalysis(id, features, renderHash);
      if (!latest.current.detached) await sync(id, s.manifest);
      debug("analysisSeconds", (performance.now() - started) / 1000);
      setAnalysis("done");
      latest.current.onChanged(s);
    })().catch((e) => {
      if (!live) return;
      setAnalysis("failed");
      latest.current.onError(e);
    });
    return () => {
      live = false;
    };
  }, [analysis, id, lanesReady]);

  useEffect(() => {
    debug("project", {
      id,
      name: m.project.name,
      sourceSha: m.source.sha256,
      stackHash: summary.state.stack_hash,
      steps: summary.state.steps.length,
      events: events.length,
      readOnly,
      detached,
      ok: (summary.report.problems ?? []).length === 0,
      parent: m.lineage?.parent_project ?? null,
      problems: summary.report.problems ?? [],
      lineageVerified: "lineage" in summary.report ? summary.report.lineage.map((l) => l.verification.ok) : [],
    });
  }, [id, m, summary, events, readOnly, detached]);

  // The view is saved to the manifest shortly after it stops changing, and
  // straight away when another project is opened.
  const pendingSave = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saveView = useCallback(async () => {
    pendingSave.current = null;
    // A step's own monitor lasts while it is selected; the saved view says Processed.
    const v = viewRef.current.monitor.startsWith("step:") ? { ...viewRef.current, monitor: "stack" as Which } : viewRef.current;
    await core.setView(id, v);
    if (!detached) await sync(id, m);
    debug("viewSaved", v);
  }, [id, m, detached]);

  const firstView = useRef(true);
  useEffect(() => {
    debug("view", view);
    if (firstView.current) {
      firstView.current = false;
      debug("viewSaved", view);
      return;
    }
    if (readOnly) return;
    if (pendingSave.current) clearTimeout(pendingSave.current);
    pendingSave.current = setTimeout(() => saveView().catch(onError), 300);
  }, [view, readOnly, saveView, onError]);

  useEffect(() => {
    flushRef.current = async () => {
      if (!pendingSave.current) return;
      clearTimeout(pendingSave.current);
      await saveView();
    };
    return () => {
      flushRef.current = null;
    };
  }, [flushRef, saveView]);

  useEffect(() => {
    let live = true;
    core.editMap(id).then((m) => {
      if (!live) return;
      setEditMap(m);
      debug("editMap", m);
    }, onError);
    return () => {
      live = false;
    };
  }, [id, summary.state.stack_hash, onError]);

  // Load what the monitor plays. Playback continues from the same place.
  useEffect(() => {
    if (!lanesReady || !editMap) return;
    let live = true;
    const p = player.current;
    const was = p.playing ? p.position() : null;
    // The processed monitor plays what will be exported: removed stretches
    // skipped, inserted silence heard. The lanes keep the original's timeline.
    const edited = view.monitor === "stack" && editMap.edited;
    core
      .pcm(id, edited ? "output" : view.monitor)
      .then((pcm) => {
        if (!live) return;
        p.load(pcm, 0, edited ? new TimeMap(editMap.pieces) : null);
        debug("monitorLoaded", view.monitor);
        if (was !== null && !dictatingRef.current) void p.play(was).then(() => setPlaying(true));
        else setPlaying(false);
      })
      .catch(onError);
    return () => {
      live = false;
    };
  }, [id, view.monitor, onError, lanesReady, editMap]);

  useEffect(() => {
    const p = player.current;
    p.onEnded = () => {
      setPlaying(false);
      setPlayhead(p.position());
    };
    return () => p.stop();
  }, []);

  useEffect(() => {
    debug("playing", playing);
    if (!playing) return;
    let raf = 0;
    const tick = () => {
      const t = player.current.position();
      setPlayhead(t);
      debug("playhead", t);
      if (t >= viewRef.current.t1) setView((v) => follow(v, t, duration));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing, duration]);

  useEffect(() => debug("playhead", playhead), [playhead]);
  useEffect(() => debug("dictating", dictating), [dictating]);

  const selectionSpan = view.selection ?? (view.tfSelection ? { t0: view.tfSelection.t0, t1: view.tfSelection.t1 } : null);

  const play = useCallback(async (from: number, to?: number, looping = false) => {
    if (dictatingRef.current) return;
    playFrom.current = from;
    await player.current.play(from, to, looping);
    setPlaying(true);
  }, []);

  const onDictating = useCallback((on: boolean) => {
    dictatingRef.current = on;
    setDictating(on);
    const p = player.current;
    if (on && p.playing) {
      const t = p.position();
      p.stop();
      setPlaying(false);
      setPlayhead(t);
    }
  }, []);

  const togglePlay = useCallback(() => {
    const p = player.current;
    if (p.playing) {
      const t = p.position();
      p.stop();
      setPlaying(false);
      setPlayhead(t);
    } else if (loop && selectionSpan) {
      void play(selectionSpan.t0, selectionSpan.t1, true);
    } else {
      void play(playhead >= duration - 1e-3 ? 0 : playhead);
    }
  }, [loop, selectionSpan, play, playhead, duration]);

  const stop = () => {
    player.current.stop();
    setPlaying(false);
    setPlayhead(playFrom.current);
  };

  const seek = (t: number) => {
    const at = Math.min(Math.max(0, t), duration);
    setPlayhead(at);
    if (player.current.playing) void play(at);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA" || tag === "BUTTON") return;
      if (e.code === "Space") {
        e.preventDefault();
        togglePlay();
      } else if (e.code === "Home") {
        seek(0);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const update = (patch: Partial<ViewState>) => setView((v) => ({ ...v, ...patch }));

  const fitRequest = useRef(0);
  const fitHeight = async () => {
    const n = ++fitRequest.current;
    try {
      const pk = await core.peaks(id, view.monitor, 0, duration, 1);
      if (n !== fitRequest.current) return; // a later request supersedes this one
      const peak = Math.max(Math.abs(pk[0] ?? 0), Math.abs(pk[1] ?? 0));
      update({ vZoom: peak > 0 ? Math.min(16384, Math.max(1, 0.9 / peak)) : 1 });
    } catch (e) {
      onError(e);
    }
  };

  const clone = async (atStep?: string) => {
    setBusy("Cloning…");
    try {
      const child = await core.cloneProject(id, atStep);
      if (!detached) {
        await sync(id, m); // the parent's log records the clone
        await sync(child.id, child.manifest);
      }
      onOpenClone(child);
    } catch (e) {
      onError(e);
    } finally {
      setBusy(null);
    }
  };

  const saveBundle = async () => {
    setBusy("Packing…");
    try {
      const bytes = await core.exportBundle(id);
      if (!detached) await sync(id, m);
      download(bytes, `${m.project.name}.nlae`);
      debug("bundleSaved", { bytes: bytes.length });
      onChanged(await core.summary(id));
    } catch (e) {
      onError(e);
    } finally {
      setBusy(null);
    }
  };

  // "Play the residual" and the like.
  const listen = (which: Which) => update({ monitor: which });

  const save = useCallback(() => (detached ? Promise.resolve() : sync(id, m)), [detached, id, m]);

  const act = async (label: string, f: () => Promise<void>) => {
    setBusy(label);
    try {
      await f();
    } catch (e) {
      onError(e);
    } finally {
      setBusy(null);
    }
  };

  const askConsole = (words: string) => setConsoleRequest((r) => ({ words, n: (r?.n ?? 0) + 1 }));

  // Removed stretches (shaded) and inserted silences (marked) on the lanes.
  const overlay = (() => {
    if (width <= 0) return [];
    const span = view.t1 - view.t0;
    const x = (t: number) => ((t - view.t0) / span) * width;
    const items: { kind: "removed" | "inserted"; from: number; to: number; label: string }[] = [];
    for (const m of editMap?.marks ?? []) {
      if (m.kind === "removed") items.push({ kind: "removed", from: m.from_s, to: m.to_s, label: `removed ${m.duration_s.toFixed(3)} s` });
      else items.push({ kind: "inserted", from: m.at_s, to: m.at_s, label: `+${m.duration_s.toFixed(3)} s silence` });
    }
    return items.map((it) => ({ ...it, x0: x(it.from), x1: x(it.to) })).filter((it) => it.x1 >= 0 && it.x0 <= width);
  })();

  // Changes to the stack. The steps above a change keep their values and are
  // rendered again, unless their input did not change.
  const rendering = (stepId: string, including: boolean) => {
    const entries = summary.state.entries;
    const at = entries.findIndex((e) => e.step_id === stepId);
    const n = entries.slice(at + 1).filter((e) => e.active).length + (including ? 1 : 0);
    return n > 0 ? `Re-rendering ${n} step${n === 1 ? "" : "s"}…` : "Updating the stack…";
  };
  const change = (label: string, f: () => Promise<ProjectSummary>) =>
    act(label, async () => {
      const s = await f();
      await save();
      onChanged(s);
    });
  const actions: StackActions | undefined = readOnly
    ? undefined
    : {
        remove: (sid, reason) => change(rendering(sid, false), () => core.exclude(id, [sid], reason)),
        restore: (sid) => change(rendering(sid, true), () => core.restore(id, [sid])),
        edit: (sid, changes) => change(rendering(sid, true), () => core.editStep(id, sid, changes)),
        remeasure: (sid) => change(rendering(sid, true), () => core.remeasure(id, sid)),
        remeasureDiff: (sid) => core.remeasureDiff(id, sid),
        undo: () => change("Undoing…", async () => (await core.undo(id)).summary),
        redo: () => change("Redoing…", async () => (await core.redo(id)).summary),
      };

  const rate = (overall: number) =>
    act("Recording the rating…", async () => {
      const s = await core.rateStack(id, overall);
      await save();
      onChanged(s);
      debug("rated", overall);
    });

  const exportWav = () =>
    act("Rendering…", async () => {
      const bytes = await core.exportWav(id, exportFormat);
      await save();
      download(bytes, `${m.project.name}.wav`, "audio/wav");
      debug("exported", { bytes: bytes.length, format: exportFormat });
      onChanged(await core.summary(id));
    });

  const selection =
    view.selection || view.tfSelection
      ? {
          time: view.selection ? ([view.selection.t0, view.selection.t1] as [number, number]) : null,
          area: view.tfSelection
            ? ([view.tfSelection.t0, view.tfSelection.t1, view.tfSelection.f_lo, view.tfSelection.f_hi] as [number, number, number, number])
            : null,
        }
      : null;

  const consoleUnavailable = readOnly
    ? "This project did not verify, so it cannot be changed."
    : analysis === "running"
      ? "Waiting for the analysis…"
      : analysis === "failed"
        ? "The analysis failed; reopen the project to try again."
        : null;

  const failure: LogFailure | null = "log" in summary.report ? (summary.report.log.first_failure as unknown as LogFailure | null) : null;

  const capture = useMemo(() => {
    const ev = events.find((e) => e.type === "source.recorded") as { data?: { capture?: Capture } } | undefined;
    return ev?.data?.capture ?? null;
  }, [events]);

  const lineage = m.lineage;
  const forkIndex = lineage?.forked_at_step ? summary.state.steps.findIndex((s) => s.step_id === lineage.forked_at_step) : -1;

  // The selected step's own monitors (not for time edits: the lanes mark those).
  const selectedStep = selected ? (summary.state.steps.find((s) => s.step_id === selected) ?? null) : null;
  const monitors =
    selectedStep && !isTimeEdit(selectedStep.op) ? [...MONITORS, ...stepMonitors(selectedStep.step_id, selectedStep.op)] : MONITORS;
  const stepMonitor = view.monitor.startsWith("step:");
  const monitorOk = !stepMonitor || monitors.some((mo) => mo.which === view.monitor);
  useEffect(() => {
    // A step removed from the stack is no longer selected; its monitors go with it.
    if (selected && !summary.state.steps.some((s) => s.step_id === selected)) setSelected(null);
    if (!monitorOk) setView((v) => ({ ...v, monitor: "stack" }));
  }, [selected, summary.state, monitorOk]);

  const laneProps = {
    projectId: id,
    which: view.monitor,
    view,
    width,
    duration,
    sampleRate,
    playhead,
    onSeek: seek,
    onSelect: (selection: ViewState["selection"]) => update({ selection }),
    onSelectTf: (tfSelection: ViewState["tfSelection"]) => update({ tfSelection }),
    onZoom: (factor: number, around: number) => setView((v) => zoom(v, factor, around, duration)),
    onScroll: (dt: number) => setView((v) => scroll(v, dt, duration)),
    renderKey: summary.state.stack_hash,
    onComplete: () => setLanesReady(true),
  };

  return (
    <div className="editor" data-testid="editor" data-project={id}>
      <div className="editor-head">
        <div>
          <h2 data-testid="project-name">{m.project.name}</h2>
          <div className="muted small-text">
            {m.source.filename} · {info.container}/{info.codec} · {sampleRate} Hz · {info.channels} ch
            {info.bits_per_sample ? ` · ${info.bits_per_sample}-bit` : ""} · {duration.toFixed(3)} s
          </div>
          {lineage && (
            <div className="lineage" data-testid="lineage">
              Clone of <b>{lineage.parent_name}</b>{" "}
              {lineage.forked_at_step
                ? `after step ${forkIndex + 1}${forkIndex >= 0 ? ` (${summary.state.steps[forkIndex].op})` : ""}`
                : "before any step"}
              {lineage.ancestors.length > 1 ? ` · ${lineage.ancestors.length} generations` : ""}
            </div>
          )}
        </div>
        <div className="head-actions">
          {analysis === "running" && (
            <span
              className="analysing"
              data-testid="analysing"
              title="Measuring levels, noise, lines and pauses; the lanes can be used meanwhile"
            >
              Analysing…
            </span>
          )}
          <IntegrityBadge report={summary.report} events={events.length} />
          <select
            value={exportFormat}
            onChange={(e) => setExportFormat(e.target.value as ExportFormat)}
            data-testid="export-format"
            title="Sample format of the exported file"
          >
            <option value="f32">32-bit float</option>
            <option value="pcm24">24-bit</option>
            <option value="pcm16">16-bit</option>
          </select>
          <button
            onClick={exportWav}
            disabled={!!readOnly || !!busy}
            data-testid="export-wav"
            title="The stack as it stands, with the final limiter, as a WAV file. Exporting approves the stack; it is logged with its hashes"
          >
            Export WAV
          </button>
          <button
            onClick={saveBundle}
            disabled={!!readOnly || !!busy}
            data-testid="save-bundle"
            title="Download a portable .nlae bundle (logged)"
          >
            Save bundle
          </button>
          <button onClick={onClose} data-testid="close-project">
            Close
          </button>
        </div>
      </div>

      {readOnly && (
        <div className="banner bad" data-testid="read-only">
          This project did not verify, so it is open read-only: nothing will be written to it. {readOnly}
        </div>
      )}
      {detached && !readOnly && (
        <div className="banner" data-testid="detached">
          Open from a bundle without a library copy; changes to the view are not kept.
        </div>
      )}
      {notice && (
        <div className="banner" data-testid="notice">
          {notice}
        </div>
      )}
      {busy && <div className="banner">{busy}</div>}

      <div className="toolbar">
        <div className="group">
          <button
            onClick={togglePlay}
            disabled={dictating}
            data-testid="play"
            title={dictating ? "Playback waits while the microphone is on" : "Play / pause (space)"}
          >
            {playing ? "❚❚ Pause" : "▶ Play"}
          </button>
          <button onClick={stop} data-testid="stop" title="Stop and return to where playback started">
            ■ Stop
          </button>
          <button
            onClick={() => selectionSpan && play(selectionSpan.t0, selectionSpan.t1, loop)}
            disabled={!selectionSpan || dictating}
            data-testid="play-selection"
          >
            ▶ Selection
          </button>
          <label className="check">
            <input type="checkbox" checked={loop} onChange={(e) => setLoop(e.target.checked)} data-testid="loop" /> Loop
          </label>
          <span className="time mono" data-testid="position">
            {fmtTime(playhead)}
          </span>
        </div>
        <div className="group" role="radiogroup" aria-label="Monitor">
          {monitors.map((mo) => (
            <button
              key={mo.which}
              className={view.monitor === mo.which ? "on" : ""}
              onClick={() => update({ monitor: mo.which })}
              title={mo.title}
              data-testid={mo.testId}
              aria-pressed={view.monitor === mo.which}
            >
              {mo.label}
            </button>
          ))}
        </div>
        <div className="group">
          <button onClick={() => setView((v) => zoom(v, 0.5, (v.t0 + v.t1) / 2, duration))} data-testid="zoom-in" title="Zoom in (wheel)">
            ＋
          </button>
          <button onClick={() => setView((v) => zoom(v, 2, (v.t0 + v.t1) / 2, duration))} data-testid="zoom-out" title="Zoom out (wheel)">
            －
          </button>
          <button onClick={() => setView((v) => fit(v, duration))} data-testid="zoom-fit">
            Fit
          </button>
          <button
            onClick={() => selectionSpan && setView((v) => showRange(v, selectionSpan.t0, selectionSpan.t1, duration))}
            disabled={!selectionSpan}
            data-testid="zoom-selection"
          >
            Zoom to selection
          </button>
        </div>
      </div>

      <div className="toolbar">
        <div className="group">
          <label>
            Frequency{" "}
            <select value={view.scale} onChange={(e) => update({ scale: e.target.value as ViewState["scale"] })} data-testid="freq-scale">
              <option value="linear">linear</option>
              <option value="log">log</option>
            </select>
          </label>
          <label>
            up to{" "}
            <select value={view.fMax} onChange={(e) => update({ fMax: Number(e.target.value) })} data-testid="fmax">
              {[2000, 4000, 8000, 12000, sampleRate / 2]
                .filter((f, i, a) => f <= sampleRate / 2 && a.indexOf(f) === i)
                .map((f) => (
                  <option key={f} value={f}>
                    {f >= 1000 ? `${f / 1000} kHz` : `${f} Hz`}
                  </option>
                ))}
            </select>
          </label>
          <label>
            Range{" "}
            <select value={view.dbRange} onChange={(e) => update({ dbRange: Number(e.target.value) })} data-testid="db-range">
              {DB_RANGES.map((r) => (
                <option key={r} value={r}>
                  {r} dB
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="group">
          <label title="Enlarges the waveform on screen only; the audio is not changed">
            Waveform height ×{view.vZoom < 10 ? view.vZoom.toFixed(1) : Math.round(view.vZoom)}{" "}
            <input
              type="range"
              min={0}
              max={14}
              step={0.25}
              value={Math.log2(view.vZoom)}
              onChange={(e) => update({ vZoom: Math.pow(2, Number(e.target.value)) })}
              data-testid="vzoom"
            />
          </label>
          <button onClick={fitHeight} data-testid="vzoom-fit">
            Fit height
          </button>
        </div>
      </div>

      <div className="lanes" ref={lanes}>
        {width > 0 && (
          <>
            <Waveform {...laneProps} />
            <Spectrogram {...laneProps} />
            <TimeAxis view={view} width={width} />
            {overlay.length > 0 && (
              <div className="edit-overlay" aria-hidden="true">
                {overlay.map((it, i) =>
                  it.kind === "removed" ? (
                    <div
                      key={i}
                      className="edit-removed"
                      style={{ left: Math.max(0, it.x0), width: Math.max(1, Math.min(width, it.x1) - Math.max(0, it.x0)) }}
                      data-testid="edit-removed"
                    >
                      <span>{it.label}</span>
                    </div>
                  ) : (
                    <div key={i} className="edit-inserted" style={{ left: it.x0 }} data-testid="edit-inserted">
                      <span>{it.label}</span>
                    </div>
                  ),
                )}
              </div>
            )}
          </>
        )}
      </div>

      <div className="selections">
        <span data-testid="selection">
          {view.selection
            ? `Time ${view.selection.t0.toFixed(3)}–${view.selection.t1.toFixed(3)} s (${(view.selection.t1 - view.selection.t0).toFixed(3)} s)`
            : "Drag on the waveform to select a time range."}
        </span>
        {view.selection && (
          <>
            <button className="small" onClick={() => update({ selection: null })}>
              clear
            </button>
            <button
              className="small"
              disabled={!!consoleUnavailable}
              onClick={() => askConsole(`remove ${view.selection!.t0.toFixed(3)} to ${view.selection!.t1.toFixed(3)} s`)}
              data-testid="remove-stretch"
              title="Leave this stretch out of the output (the original is untouched)"
            >
              Remove this stretch
            </button>
          </>
        )}
        <span data-testid="tf-selection">
          {view.tfSelection
            ? `Area ${view.tfSelection.t0.toFixed(3)}–${view.tfSelection.t1.toFixed(3)} s × ${view.tfSelection.f_lo}–${view.tfSelection.f_hi} Hz`
            : "Drag on the spectrogram to select a time × frequency area."}
        </span>
        {view.tfSelection && (
          <button className="small" onClick={() => update({ tfSelection: null })}>
            clear
          </button>
        )}
        <span className="insert-silence">
          Insert{" "}
          <input
            type="number"
            min={0.001}
            max={3600}
            step={0.1}
            value={insertLength}
            onChange={(e) => setInsertLength(Number(e.target.value))}
            data-testid="insert-length"
          />{" "}
          s of silence{" "}
          <button
            className="small"
            disabled={!!consoleUnavailable || !(insertLength > 0)}
            onClick={() => askConsole(`insert ${insertLength} s of silence at ${playhead.toFixed(3)} s`)}
            data-testid="insert-silence"
            title="At the playhead: click the waveform to place it"
          >
            at the playhead ({fmtTime(playhead)})
          </button>
        </span>
      </div>

      <Console
        summary={summary}
        selection={selection}
        unavailable={consoleUnavailable}
        onSummary={onChanged}
        onListen={listen}
        onSelectStep={setSelected}
        onError={onError}
        save={save}
        request={consoleRequest}
        onDictating={onDictating}
      />

      <div className="panels">
        <StackPanel
          state={summary.state}
          onClone={clone}
          canClone={!readOnly && !busy}
          brokenAtLine={failure ? Number(failure.line) : null}
          actions={actions}
          busy={!!busy}
          selected={selected}
          onSelect={setSelected}
          descriptors={descriptors}
          onRate={readOnly || busy ? undefined : rate}
          output={editMap?.edited ? { duration: editMap.duration_s, original: editMap.original_duration_s } : null}
        />
        <div className="panel">
          <div className="panel-head">
            <h3>Record</h3>
          </div>
          <dl className="facts">
            <dt>Source SHA-256</dt>
            <dd className="mono" data-testid="source-sha" title={m.source.sha256}>
              {m.source.sha256.slice(7, 23)}…
            </dd>
            <dt>Stack hash</dt>
            <dd className="mono" data-testid="stack-hash" title={summary.state.stack_hash}>
              {summary.state.stack_hash.slice(7, 23)}…
            </dd>
            <dt>Render hash</dt>
            <dd>
              {renderHash && renderHash.which === view.monitor ? (
                <span className="mono" data-testid="render-hash" data-which={renderHash.which} title={renderHash.hash}>
                  {renderHash.hash.slice(7, 23)}…
                </span>
              ) : (
                <button
                  className="small"
                  data-testid="compute-render-hash"
                  title="SHA-256 of the samples the monitor plays; native builds record the same hash for each step's output"
                  onClick={async () => {
                    try {
                      const which = view.monitor;
                      setRenderHash({ which, hash: await core.renderHash(id, which) });
                    } catch (e) {
                      onError(e);
                    }
                  }}
                >
                  compute ({monitors.find((x) => x.which === view.monitor)?.label})
                </button>
              )}
            </dd>
            <dt>Project</dt>
            <dd className="mono">{id}</dd>
            <dt>Created</dt>
            <dd>{m.project.created}</dd>
          </dl>
          {capture && (
            <div className="capture" data-testid="capture">
              <h4>Recorded in the app</h4>
              <table>
                <thead>
                  <tr>
                    <th>setting</th>
                    <th>requested</th>
                    <th>applied by the browser</th>
                  </tr>
                </thead>
                <tbody>
                  {["echoCancellation", "noiseSuppression", "autoGainControl", "channelCount", "sampleRate"].map((k) => (
                    <tr key={k} data-testid={`capture-${k}`}>
                      <td>{k}</td>
                      <td>{String(capture.requested?.[k] ?? "—")}</td>
                      <td>{String(capture.applied?.[k] ?? "not reported")}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="muted small-text">
                Device: {capture.device_label || "unnamed"} · context rate {capture.context_sample_rate} Hz
              </div>
            </div>
          )}
        </div>
        <LogViewer events={events} failure={failure} />
      </div>
    </div>
  );
}
