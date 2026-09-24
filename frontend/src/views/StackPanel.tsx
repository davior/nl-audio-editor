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
    default:
      return "";
  }
}

/** The stack, read-only in M0. Steps are added from the command line (the console comes in M1). */
export function StackPanel({ state, onClone, canClone }: { state: StackState; onClone: (atStep?: string) => void; canClone: boolean }) {
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
      {state.steps.length === 0 && <div className="muted">No steps yet. The original is untouched.</div>}
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
            <div className="step-meta">
              {s.actor.model ? `${s.actor.model}` : s.actor.kind} via {s.origin}
              {s.intent ? ` — “${s.intent}”` : ""}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}
