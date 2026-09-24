// The console: say what you want. Routine requests are handled locally; the
// rest go to the chosen model with words and analysis numbers only. Every
// proposal is previewed on a short window and waits for Accept or Reject;
// values can be changed first (recorded as a modification).
import { useEffect, useMemo, useRef, useState } from "react";
import { complete, loadConfig, type ProviderConfig } from "../assistant/provider";
import { core } from "../core/client";
import type { PreviewRecord, PreviewResult, ProjectSummary, Selection, Step, Turn, Which } from "../core/types";
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
}

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

export function Console({ summary, selection, unavailable, onSummary, onListen, onPreview, onError, save }: Props) {
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

  const ask = async () => {
    const w = words.trim();
    if (!w || busy) return;
    const tid = next.current++;
    const open = turns.some((t) => t.status === "proposal");
    setTurns((ts) => [...ts, { id: tid, words: w, via: null, status: "working" }]);
    setWords("");
    setBusy(true);
    try {
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
          await core.previewRecipe(id, route.name, w),
          "local",
          `The built-in ${route.name} recipe, measured on this recording.`,
        );
      } else if (route.route === "steps") {
        await showPreview(tid, await core.previewRouted(id, route.steps, w), "local", route.steps.map((s) => s.understood).join(" "));
      } else {
        const config = loadConfig();
        update(tid, { via: "model", model: `${config.provider} · ${config.model}`, text: `Asking ${config.model}…` });
        let body = await core.assistantRequest(id, config.model, history, w, selection);
        let { response, latencyMs, host } = await complete(config, body);
        let parsed = await core.parseResponse(JSON.stringify(response));
        const exchange = await core.recordExchange(id, {
          provider: config.provider,
          model: config.model,
          host,
          request: body,
          response,
          latency_ms: latencyMs,
          problems: parsed.problems ?? null,
        });
        if (parsed.problems) {
          // One chance to correct itself.
          body = await core.correctionRequest(body, JSON.stringify(response), parsed.problems);
          ({ response, latencyMs, host } = await complete(config, body));
          parsed = await core.parseResponse(JSON.stringify(response));
          await core.recordExchange(id, {
            provider: config.provider,
            model: config.model,
            host,
            request: body,
            response,
            latency_ms: latencyMs,
            problems: parsed.problems ?? null,
            corrects: exchange,
          });
        }
        if (parsed.problems) {
          await save();
          update(tid, { status: "error", problems: parsed.problems, text: "The model's proposal could not be used." });
        } else if (parsed.proposal!.steps.length === 0) {
          await save();
          update(tid, { status: "reply", text: parsed.proposal!.text ?? "" });
        } else {
          await showPreview(
            tid,
            await core.previewProposal(id, parsed.proposal!, w, config.model, config.provider, exchange),
            "model",
            parsed.proposal!.text ?? undefined,
          );
        }
      }
    } catch (e) {
      update(tid, { status: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

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
          <div key={t.id} className={`turn ${t.status}`} data-testid="turn" data-status={t.status} data-via={t.via ?? ""}>
            <div className="you">{t.words}</div>
            {t.via && <div className="via">{t.via === "local" ? "handled here" : `via ${t.model}`}</div>}
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
          void ask();
        }}
      >
        <input
          value={words}
          onChange={(e) => setWords(e.target.value)}
          placeholder={unavailable ?? "What should happen?"}
          disabled={!!unavailable || busy}
          data-testid="console-input"
        />
        <button type="submit" disabled={!!unavailable || busy || !words.trim()} data-testid="console-send">
          {busy ? "…" : "Send"}
        </button>
      </form>
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
