// The console: say what you want, typed or spoken. Routine requests are
// handled locally; the rest go to the chosen model with words and analysis
// numbers only. Every proposal is previewed on a short window and waits for
// Accept or Reject; values can be changed first (recorded as a modification).
// Spoken words fill the box as they are recognised and are sent, like typed
// ones, with Enter; what was heard is logged next to what was sent.
import { useEffect, useMemo, useRef, useState } from "react";
import { compose, Listening, loadSpeechConfig, SPEECH_PROVIDER, type Listened } from "../assistant/dictation";
import { complete, loadConfig, type ProviderConfig } from "../assistant/provider";
import { core } from "../core/client";
import type { PreviewRecord, PreviewResult, ProjectSummary, Segment, Selection, Step, Turn, Which } from "../core/types";
import { debug } from "../debug";
import { Settings } from "./Settings";
import { describe } from "./StackPanel";

export interface PreviewInfo {
  record: PreviewRecord;
  window: [number, number];
}

interface ConsoleTurn {
  id: number;
  words: string;
  via: "local" | "model" | null;
  /** The model asked, when one was. */
  model?: string;
  status: "working" | "proposal" | "reply" | "done" | "error";
  text?: string;
  problems?: string[];
  preview?: PreviewRecord;
  outcome?: string;
  /** The words were dictated (possibly changed before sending). */
  spoken?: boolean;
}

/** The dictation behind the words in the box, until they are sent or cleared. */
interface Spoken {
  model: string;
  host: string;
  params: [string, string][];
  requestIds: string[];
  segments: Segment[];
  audioS: number;
  latencyMs: number;
}

type Mic = "off" | "starting" | "listening" | "finishing";

/** Listening stops by itself after this long, or this long without speech. */
const MAX_LISTEN_MS = 30_000;
const NO_SPEECH_MS = 8_000;

interface Props {
  summary: ProjectSummary;
  selection: Selection | null;
  /** Why the console cannot be used right now, if it cannot. */
  unavailable: string | null;
  onSummary: (s: ProjectSummary) => void;
  onListen: (which: Which) => void;
  /** A preview opened, or closed (`accepted` when it went onto the stack). */
  onPreview: (p: PreviewInfo | null, accepted?: boolean) => void;
  onError: (e: unknown) => void;
  save: () => Promise<void>;
  /** A request made with a button elsewhere; handled as if typed (each new `n` once). */
  request?: { words: string; n: number } | null;
  /** The microphone opened (true) or closed: playback pauses meanwhile, so no recording is streamed. */
  onDictating: (on: boolean) => void;
}

type Descriptor = {
  id: string;
  version: number;
  params: { id: string; title: string; type: string; min?: number; max?: number; step?: number; values?: string[]; unit?: string }[];
};

function scopeWords(s: Step["scope"]): string {
  switch (s.kind) {
    case "clip":
      return "whole recording";
    case "time_range":
      return `${s.t0.toFixed(2)}–${s.t1.toFixed(2)} s`;
    case "band":
      return `${s.f_lo.toFixed(0)}–${s.f_hi.toFixed(0)} Hz`;
    case "tf_patch":
      return `${s.t0.toFixed(2)}–${s.t1.toFixed(2)} s × ${s.f_lo.toFixed(0)}–${s.f_hi.toFixed(0)} Hz`;
  }
}

function measurementText(m: Record<string, unknown>): string {
  return Object.entries(m)
    .filter(([, v]) => typeof v === "number")
    .slice(0, 4)
    .map(([k, v]) => `${k.replace(/_/g, " ")} ${(v as number).toFixed(2)}`)
    .join(" · ");
}

/** A value editor for one parameter; numbers, choices and switches only. */
function ParamInput({ spec, value, onChange }: { spec: Descriptor["params"][number]; value: unknown; onChange: (v: unknown) => void }) {
  if (spec.type === "number" || spec.type === "integer") {
    if (typeof value !== "number") return <span className="muted">{String(value)}</span>;
    return (
      <input
        type="number"
        value={value}
        min={spec.min}
        max={spec.max}
        step={spec.step ?? (spec.type === "integer" ? 1 : 0.1)}
        onChange={(e) => onChange(Number(e.target.value))}
        data-testid={`param-${spec.id}`}
      />
    );
  }
  if (spec.type === "enum")
    return (
      <select value={String(value)} onChange={(e) => onChange(e.target.value)} data-testid={`param-${spec.id}`}>
        {spec.values?.map((v) => (
          <option key={v}>{v}</option>
        ))}
      </select>
    );
  if (spec.type === "bool")
    return <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} data-testid={`param-${spec.id}`} />;
  return <span className="muted">{JSON.stringify(value).slice(0, 40)}</span>;
}

function ProposalCard({
  turn,
  descriptors,
  busy,
  onAccept,
  onReject,
}: {
  turn: ConsoleTurn;
  descriptors: Descriptor[];
  busy: boolean;
  onAccept: (overrides: Record<number, Record<string, unknown>>, disabled: number[]) => void;
  onReject: (reason: string) => void;
}) {
  const pv = turn.preview!;
  const [editing, setEditing] = useState<number | null>(null);
  const [overrides, setOverrides] = useState<Record<number, Record<string, unknown>>>({});
  const [off, setOff] = useState<number[]>([]);
  const [reason, setReason] = useState("");
  const [rejecting, setRejecting] = useState(false);
  return (
    <div className="proposal" data-testid="proposal">
      {pv.steps.map((s, i) => {
        const desc = descriptors.find((d) => d.id === s.op && d.version === s.op_version);
        const changed = overrides[i] ?? {};
        return (
          <div key={s.step_id} className={`proposal-step ${off.includes(i) ? "off" : ""}`} data-testid="proposal-step" data-op={s.op}>
            <div className="step-top">
              {pv.kind === "plan" && (
                <input
                  type="checkbox"
                  checked={!off.includes(i)}
                  onChange={(e) => setOff((o) => (e.target.checked ? o.filter((x) => x !== i) : [...o, i]))}
                  title="Include this step"
                  data-testid="plan-step-toggle"
                />
              )}
              <span className="op">{s.op}</span>
              <span className="label">{scopeWords(s.scope)}</span>
              {desc && (
                <button className="small" onClick={() => setEditing(editing === i ? null : i)} data-testid="modify">
                  {editing === i ? "done" : "change"}
                </button>
              )}
            </div>
            <div className="step-desc">{describe(s)}</div>
            {Object.keys(changed).length > 0 && (
              <div className="step-meta" data-testid="changed">
                changed:{" "}
                {Object.entries(changed)
                  .map(([k, v]) => `${k} → ${String(v)}`)
                  .join(", ")}
              </div>
            )}
            <div className="step-meta" data-testid="measurements">
              {measurementText(s.measurements as Record<string, unknown>)}
            </div>
            {editing === i && desc && (
              <div className="param-editor">
                {desc.params
                  .filter((p) => ["number", "integer", "enum", "bool"].includes(p.type))
                  .map((p) => (
                    <label key={p.id}>
                      {p.title}
                      {p.unit ? ` (${p.unit})` : ""}
                      <ParamInput
                        spec={p}
                        value={p.id in changed ? changed[p.id] : (s.params as Record<string, unknown>)[p.id]}
                        onChange={(v) => setOverrides((o) => ({ ...o, [i]: { ...(o[i] ?? {}), [p.id]: v } }))}
                      />
                    </label>
                  ))}
              </div>
            )}
          </div>
        );
      })}
      <div className="proposal-actions">
        <button className="accept" disabled={busy} onClick={() => onAccept(overrides, off)} data-testid="accept">
          Accept
        </button>
        {!rejecting ? (
          <button disabled={busy} onClick={() => setRejecting(true)} data-testid="reject">
            Reject
          </button>
        ) : (
          <>
            <input
              placeholder="Why? (optional, recorded)"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              data-testid="reject-reason"
            />
            <button disabled={busy} onClick={() => onReject(reason)} data-testid="reject-confirm">
              Reject
            </button>
          </>
        )}
      </div>
    </div>
  );
}

export function Console({ summary, selection, unavailable, onSummary, onListen, onPreview, onError, save, request, onDictating }: Props) {
  const id = summary.id;
  const [turns, setTurns] = useState<ConsoleTurn[]>([]);
  const [words, setWords] = useState("");
  const [busy, setBusy] = useState(false);
  const [descriptors, setDescriptors] = useState<Descriptor[]>([]);
  const next = useRef(1);
  const turnsEl = useRef<HTMLDivElement>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [configVersion, setConfigVersion] = useState(0);
  const cfg = useMemo<ProviderConfig>(() => loadConfig(), [configVersion]);
  // Dictation: the words fill the box as they are recognised; nothing is sent until Enter.
  const [mic, setMic] = useState<Mic>("off");
  const [micError, setMicError] = useState<string | null>(null);
  const listening = useRef<Listening | null>(null);
  // Each opening of the microphone; cancelling (or leaving) abandons one still opening.
  const attempt = useRef(0);
  const opening = useRef<AbortController | null>(null);
  const typedBefore = useRef("");
  const spoken = useRef<Spoken | null>(null);
  const micTimers = useRef<ReturnType<typeof setTimeout>[]>([]);
  const inputEl = useRef<HTMLInputElement>(null);

  useEffect(() => {
    core.descriptors().then((d) => setDescriptors(d as Descriptor[]), onError);
  }, [onError]);

  useEffect(
    () =>
      debug(
        "console",
        turns.map((t) => ({ words: t.words, via: t.via, status: t.status, outcome: t.outcome })),
      ),
    [turns],
  );

  // The latest turn, with its Accept and Reject, stays in view.
  useEffect(() => {
    const el = turnsEl.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns]);

  const update = (tid: number, patch: Partial<ConsoleTurn>) => setTurns((ts) => ts.map((t) => (t.id === tid ? { ...t, ...patch } : t)));

  const history: Turn[] = turns
    .filter((t) => t.status !== "working")
    .flatMap((t) => [{ role: "user", text: t.words }, ...(t.text ? [{ role: "assistant", text: t.text }] : [])]);

  const showPreview = async (tid: number, r: PreviewResult, via: "local" | "model", text?: string) => {
    await save();
    onSummary(r.summary);
    update(tid, { status: "proposal", preview: r.record, via, text });
    onPreview({ record: r.record, window: r.window });
  };

  const clearMicTimers = () => {
    micTimers.current.forEach(clearTimeout);
    micTimers.current = [];
  };

  const endListening = () => {
    clearMicTimers();
    setMic("off");
    onDictating(false);
    inputEl.current?.focus();
  };

  /** Keep what a listening session heard with the words in the box. */
  const keepHeard = (r: Listened) => {
    setWords(compose(typedBefore.current, r.heard.finals));
    if (r.heard.finals.length === 0) return;
    const s = spoken.current ?? {
      model: r.params.find(([k]) => k === "model")?.[1] ?? loadSpeechConfig().model,
      host: r.host,
      params: r.params,
      requestIds: [],
      segments: [],
      audioS: 0,
      latencyMs: 0,
    };
    s.segments.push(...r.heard.finals);
    if (r.heard.requestId) s.requestIds.push(r.heard.requestId);
    s.audioS += r.audioS;
    s.latencyMs = r.latencyMs;
    spoken.current = s;
  };

  const finishListening = async () => {
    const l = listening.current;
    if (!l) return;
    listening.current = null;
    setMic("finishing");
    clearMicTimers();
    try {
      keepHeard(await l.finish());
    } catch (e) {
      setMicError(e instanceof Error ? e.message : String(e));
    } finally {
      endListening();
    }
  };

  /** Stop at once; the box goes back to what was typed before. */
  const cancelListening = () => {
    attempt.current++;
    opening.current?.abort();
    const l = listening.current;
    listening.current = null;
    l?.cancel();
    setWords(typedBefore.current);
    endListening();
  };

  const startListening = async () => {
    if (listening.current || mic !== "off") return;
    const speech = loadSpeechConfig();
    if (!speech.key.trim()) {
      setMicError("Add a Deepgram key in the settings (Speech) to dictate.");
      setSettingsOpen(true);
      return;
    }
    setMicError(null);
    typedBefore.current = words;
    setMic("starting");
    onDictating(true);
    const n = ++attempt.current;
    const current = () => n === attempt.current;
    const ac = new AbortController();
    opening.current = ac;
    try {
      const l = await Listening.open(
        speech,
        (rate) => core.listenParams(speech.model, speech.language, rate, speech.optOut),
        {
          onHeard: (h) => {
            if (current()) setWords(compose(typedBefore.current, h.finals, h.interim));
          },
          onPause: () => {
            if (current()) void finishListening();
          },
          onFailure: (message, r) => {
            if (!current()) return;
            listening.current = null;
            keepHeard(r);
            setMicError(message);
            endListening();
          },
        },
        ac.signal,
      );
      if (!current()) {
        // Cancelled, or the console closed, while the stream was opening.
        l.cancel();
        return;
      }
      listening.current = l;
      setMic("listening");
      micTimers.current = [
        setTimeout(() => void finishListening(), MAX_LISTEN_MS),
        setTimeout(() => {
          if (listening.current && !listening.current.speech) void finishListening();
        }, NO_SPEECH_MS),
      ];
    } catch (e) {
      if (!current()) return;
      setMicError(e instanceof Error ? e.message : String(e));
      endListening();
    } finally {
      if (opening.current === ac) opening.current = null;
    }
  };

  // Leaving the console closes the microphone.
  useEffect(
    () => () => {
      attempt.current++;
      opening.current?.abort();
      listening.current?.cancel();
      listening.current = null;
      micTimers.current.forEach(clearTimeout);
    },
    [],
  );

  const ask = async (text?: string) => {
    const w = (text ?? words).trim();
    if (!w || busy) return;
    const tid = next.current++;
    const open = turns.some((t) => t.status === "proposal");
    // Words from the box may have been dictated; a request made with a button was not.
    const dictated = text === undefined ? spoken.current : null;
    if (text === undefined) spoken.current = null;
    setTurns((ts) => [...ts, { id: tid, words: w, via: null, status: "working", spoken: !!dictated }]);
    if (text === undefined) setWords("");
    setBusy(true);
    try {
      // What was heard and what is sent are logged before anything is done with them.
      let dictation: string | undefined;
      if (dictated) {
        dictation = await core.recordDictation(id, {
          provider: SPEECH_PROVIDER,
          model: dictated.model,
          host: dictated.host,
          params: dictated.params,
          request_ids: dictated.requestIds,
          segments: dictated.segments,
          words: w,
          audio_s: dictated.audioS,
          latency_ms: dictated.latencyMs,
        });
        await save();
      }
      const route = await core.route(w, selection);
      if (open && route.route !== "listen") {
        // Anything but listening sets an open proposal aside (it stays in the log, undecided).
        onPreview(null);
        setTurns((ts) => ts.map((t) => (t.status === "proposal" ? { ...t, status: "done", outcome: "set aside" } : t)));
      }
      if (route.route === "listen") {
        onListen(route.which as Which);
        update(tid, {
          via: "local",
          status: "done",
          text: `Listening to ${route.which === "stack" ? "the processed audio" : route.which === "source" ? "the original" : "what was removed"}.`,
        });
      } else if (route.route === "undo") {
        const s = await core.removeTop(id);
        await save();
        onSummary(s);
        update(tid, { via: "local", status: "done", text: "Removed the last step (it stays in the log)." });
      } else if (route.route === "recipe") {
        await showPreview(
          tid,
          await core.previewRecipe(id, route.name, w, dictation),
          "local",
          `The built-in ${route.name} recipe, measured on this recording.`,
        );
      } else if (route.route === "steps") {
        await showPreview(
          tid,
          await core.previewRouted(id, route.steps, w, dictation),
          "local",
          route.steps.map((s) => s.understood).join(" "),
        );
      } else {
        const config = loadConfig();
        update(tid, { via: "model", model: `${config.provider} · ${config.model}`, text: `Asking ${config.model}…` });
        // The core decides each round: use the answer, describe operations the
        // model asked to see, or ask once for a correction. Every exchange is logged.
        let body = await core.assistantRequest(id, config.model, history, w, selection);
        const rounds = { described: false, corrected: false };
        let link: { corrects?: string; describes?: string } = {};
        for (;;) {
          const { response, latencyMs, host } = await complete(config, body);
          const next = await core.nextRound(body, JSON.stringify(response), rounds);
          const exchange = await core.recordExchange(id, {
            provider: config.provider,
            model: config.model,
            host,
            request: body,
            response,
            latency_ms: latencyMs,
            problems: next.next === "correct" || next.next === "failed" ? next.problems : null,
            ...link,
          });
          if (next.next === "describe") {
            rounds.described = true;
            link = { describes: exchange };
            body = next.request;
            update(tid, { text: `Looking up ${next.ids.join(", ")}…` });
            continue;
          }
          if (next.next === "correct") {
            rounds.corrected = true;
            link = { corrects: exchange };
            body = next.request;
            continue;
          }
          if (next.next === "failed") {
            await save();
            update(tid, { status: "error", problems: next.problems, text: "The model's proposal could not be used." });
          } else if (next.proposal.steps.length === 0) {
            await save();
            update(tid, { status: "reply", text: next.proposal.text ?? "" });
          } else {
            await showPreview(
              tid,
              await core.previewProposal(id, next.proposal, w, config.model, config.provider, exchange, dictation),
              "model",
              next.proposal.text ?? undefined,
            );
          }
          break;
        }
      }
    } catch (e) {
      update(tid, { status: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  const askRef = useRef(ask);
  askRef.current = ask;
  useEffect(() => {
    if (request) void askRef.current(request.words);
  }, [request]);

  const decide = async (tid: number, f: () => Promise<ProjectSummary>, outcome: string) => {
    setBusy(true);
    try {
      const s = await f();
      await save();
      onSummary(s);
      onPreview(null, outcome.startsWith("accepted"));
      update(tid, { status: "done", outcome });
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel console" data-testid="console">
      <div className="panel-head">
        <h3>Console</h3>
        <button className="small" onClick={() => setSettingsOpen(true)} data-testid="assistant-settings" title="Provider, model and key">
          {cfg.model ? `${cfg.provider} · ${cfg.model}` : "choose a model"}
        </button>
      </div>
      <div className="turns" data-testid="turns" ref={turnsEl}>
        {turns.length === 0 && (
          <p className="muted small-text">
            Say what you want, e.g. “clean this recording up”, “the hum is distracting”, “cut 3,100 to 3,200 Hz by 12 dB”, or “compress the
            bangs here” with an area selected. Routine requests are handled here; the rest go to the model with words and analysis numbers
            only, never audio.
          </p>
        )}
        {turns.map((t) => (
          <div
            key={t.id}
            className={`turn ${t.status}`}
            data-testid="turn"
            data-status={t.status}
            data-via={t.via ?? ""}
            data-spoken={t.spoken ? "yes" : "no"}
          >
            <div className="you">{t.words}</div>
            {t.via && (
              <div className="via">
                {t.spoken ? "spoken · " : ""}
                {t.via === "local" ? "handled here" : `via ${t.model}`}
              </div>
            )}
            {t.text && (
              <div className="reply" data-testid="turn-text">
                {t.text}
              </div>
            )}
            {t.problems && (
              <ul className="problems" data-testid="turn-problems">
                {t.problems.map((p, i) => (
                  <li key={i}>{p}</li>
                ))}
              </ul>
            )}
            {t.status === "proposal" && t.preview && (
              <ProposalCard
                turn={t}
                descriptors={descriptors}
                busy={busy}
                onAccept={(overrides, disabled) =>
                  decide(
                    t.id,
                    () => core.accept(id, t.preview!.preview_id, overrides, disabled),
                    disabled.length ? "accepted (some steps off)" : "accepted",
                  )
                }
                onReject={(reason) => decide(t.id, () => core.reject(id, t.preview!.preview_id, reason), "rejected")}
              />
            )}
            {t.outcome && (
              <div className="outcome" data-testid="turn-outcome">
                {t.outcome}
              </div>
            )}
          </div>
        ))}
      </div>
      <form
        className="ask"
        onSubmit={(e) => {
          e.preventDefault();
          if (mic === "off") void ask();
        }}
        onKeyDown={(e) => {
          // While listening, Enter stops (the words stay in the box to be checked) and Escape discards.
          if (mic === "off") return;
          if (e.key === "Enter") {
            e.preventDefault();
            if (mic === "listening") void finishListening();
          } else if (e.key === "Escape" && (mic === "starting" || mic === "listening")) {
            e.preventDefault();
            cancelListening();
          }
        }}
      >
        <input
          ref={inputEl}
          value={words}
          onChange={(e) => {
            setWords(e.target.value);
            if (!e.target.value.trim()) spoken.current = null;
          }}
          placeholder={unavailable ?? (mic === "off" ? "What should happen?" : "Listening…")}
          disabled={!!unavailable || busy}
          readOnly={mic !== "off"}
          data-testid="console-input"
        />
        <button
          type="button"
          className={`mic ${mic}`}
          onClick={() => {
            if (mic === "off") void startListening();
            else if (mic === "listening") void finishListening();
          }}
          disabled={!!unavailable || busy || mic === "starting" || mic === "finishing"}
          aria-pressed={mic !== "off"}
          data-testid="console-mic"
          data-state={mic}
          title={
            mic === "off"
              ? "Speak your request (Deepgram). The words fill the box; press Enter to send them."
              : "Stop listening (Enter). Escape discards what was heard."
          }
        >
          {mic === "off" ? "Speak" : mic === "starting" ? "Opening…" : mic === "listening" ? "Stop" : "…"}
        </button>
        <button type="submit" disabled={!!unavailable || busy || !words.trim() || mic !== "off"} data-testid="console-send">
          {busy ? "…" : "Send"}
        </button>
      </form>
      {mic === "listening" && (
        <div className="mic-status small-text" data-testid="mic-status">
          Listening. The words appear as they are recognised; Enter or Stop when done, Escape to discard.
        </div>
      )}
      {micError && mic === "off" && (
        <div className="mic-error small-text bad" data-testid="mic-error">
          {micError}
        </div>
      )}
      {settingsOpen && (
        <Settings
          initial={cfg}
          onClose={() => {
            setSettingsOpen(false);
            setConfigVersion((v) => v + 1);
          }}
        />
      )}
    </div>
  );
}
