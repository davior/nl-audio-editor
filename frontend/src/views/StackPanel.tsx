import type { StackState, Step } from "../core/types";

export function describe(s: Step): string {
  const r = s.resolved as Record<string, unknown>;
  const n = (k: string, d = 1) => (typeof r[k] === "number" ? (r[k] as number).toFixed(d) : String(r[k] ?? ""));
  switch (s.op) {
    case "gain":
      return `${n("gain_db")} dB`;
    case "normalise":
      return `peak → ${n("target_peak_dbfs")} dBFS (${n("gain_db", 2)} dB)`;
    case "dc_remove":
      return r.mode === "mean" ? "mean offset removed" : "drift removed";
    case "noise_reduce": {
      const pr = r.profile_range as { t0: number; t1: number; auto: boolean } | undefined;
      return `profile ${pr?.t0.toFixed(2)}–${pr?.t1.toFixed(2)} s${pr?.auto ? " (quietest)" : ""}, −${n("reduction_db")} dB`;
    }
    case "line_reduce": {
      const lines = (r.resolved_lines as { freq_hz: number; depth_db: number }[] | undefined) ?? [];
      return lines.map((l) => `${l.freq_hz.toFixed(0)} Hz −${l.depth_db.toFixed(1)}`).join(", ") || "no lines";
    }
    case "band_cut":
      return `${n("f_lo", 0)}–${n("f_hi", 0)} Hz −${n("depth_db")} dB`;
    case "spectral_compressor":
      return `${String(r.mode)} mode, ratio ${n("ratio")}, max −${n("max_reduction_db")} dB`;
    case "compressor":
      return `threshold ${n("threshold_dbfs")} dBFS, ratio ${n("ratio")}`;
    case "remove_time": {
      const sc = s.scope as { t0?: number; t1?: number };
      return `removes ${sc.t0?.toFixed(3)}–${sc.t1?.toFixed(3)} s of the original (${n("removed_s", 3)} s)`;
    }
    case "insert_silence":
      return `inserts ${n("inserted_s", 3)} s of silence at ${n("at_s", 3)} s of the original`;
    default:
      return "";
  }
}

/** The stack: steps come from the console or the command line; the top step can be undone and the result rated. */
export function StackPanel({
  state,
  onClone,
  canClone,
  brokenAtLine,
  onUndo,
  onRate,
  output,
}: {
  state: StackState;
  onClone: (atStep?: string) => void;
  canClone: boolean;
  /** When the log did not verify: the stack shows only what was recorded before this line. */
  brokenAtLine: number | null;
  /** Remove the top step (it stays in the log); absent when the project cannot be changed. */
  onUndo?: () => void;
  /** Rate the stack as it stands, 1–5. */
  onRate?: (overall: number) => void;
  /** With time edits: the output's length and the original's (seconds). */
  output?: { duration: number; original: number } | null;
}) {
  return (
    <div className="panel" data-testid="stack">
      <div className="panel-head">
        <h3>Stack</h3>
        {canClone && (
          <button onClick={() => onClone()} data-testid="clone-current" title="Clone the project as it is now">
            Clone current
          </button>
        )}
      </div>
      {brokenAtLine !== null && (
        <div className="problem-note" data-testid="stack-partial">
          Only steps recorded before line {brokenAtLine} of the log are shown; nothing after it can be verified.
        </div>
      )}
      {state.steps.length === 0 && brokenAtLine === null && <div className="muted">No steps yet. The original is untouched.</div>}
      {output && (
        <div className="output-length" data-testid="output-length">
          Output {output.duration.toFixed(3)} s (original {output.original.toFixed(3)} s). Removed stretches are shaded on the lanes and
          skipped when playing <i>Processed</i>.
        </div>
      )}
      {(onUndo || onRate) && state.steps.length > 0 && (
        <div className="stack-actions">
          {onUndo && (
            <button className="small" onClick={onUndo} data-testid="undo" title="Remove the top step; it stays in the log">
              Undo last step
            </button>
          )}
          {onRate && (
            <span className="rate" title="How good is the result? Recorded for learning.">
              Rate:
              {[1, 2, 3, 4, 5].map((n) => (
                <button key={n} className="small" onClick={() => onRate(n)} data-testid={`rate-${n}`}>
                  {n}
                </button>
              ))}
            </span>
          )}
        </div>
      )}
      <ol className="steps">
        {state.steps.map((s) => (
          <li key={s.step_id} data-testid="stack-step" data-op={s.op}>
            <div className="step-top">
              <span className="op">{s.op}</span>
              {s.plan_id && <span className="tag">plan</span>}
              {s.inherited_from && <span className="tag">inherited</span>}
              <span className="label">{s.label}</span>
              {canClone && (
                <button
                  className="small"
                  onClick={() => onClone(s.step_id)}
                  title="Clone the project at this step"
                  data-testid="clone-here"
                >
                  Clone here
                </button>
              )}
            </div>
            <div className="step-desc">{describe(s)}</div>
            <div className="step-meta" data-testid="step-meta">
              {s.actor.model ? `${s.actor.model}${s.actor.provider ? ` (${s.actor.provider})` : ""}` : s.actor.kind} via {s.origin}
              {s.intent ? ` — “${s.intent}”` : ""}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}
