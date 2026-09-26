import { useState } from "react";
import type { DiffEntry, StackChange, StackState, Step } from "../core/types";
import { StepEditor, type Descriptor } from "./StepEditor";

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
    case "high_pass":
      return `below ${n("cutoff_hz", 0)} Hz, ${n("slope_db_per_octave", 0)} dB/octave`;
    case "low_pass":
      return `above ${n("cutoff_hz", 0)} Hz, ${n("slope_db_per_octave", 0)} dB/octave`;
    case "bell":
      return `${(r.gain_db as number) > 0 ? "+" : ""}${n("gain_db")} dB at ${n("freq_hz", 0)} Hz, ${n("width_octaves")} octave wide`;
    case "shelf":
      return `${String(r.kind)} shelf ${(r.gain_db as number) > 0 ? "+" : ""}${n("gain_db")} dB at ${n("freq_hz", 0)} Hz`;
    case "tilt":
      return `${(r.db_per_octave as number) > 0 ? "+" : ""}${n("db_per_octave")} dB/octave around ${n("pivot_hz", 0)} Hz`;
    case "gate":
      return `below ${n("threshold_dbfs")} dBFS, down ${n("range_db", 0)} dB`;
    case "loudness_normalise":
      return `loudness → ${n("target_lufs")} LUFS (${n("gain_db", 2)} dB)`;
    case "hum_reduce": {
      const lines = (r.resolved_lines as unknown[] | undefined) ?? [];
      return r.fundamental_hz == null
        ? "no hum found"
        : `${n("fundamental_hz", 2)} Hz hum, ${lines.length} harmonics −${n("depth_db", 0)} dB`;
    }
    default:
      return "";
  }
}

/** A step on the stack, active or excluded, in its current version. */
export function stepOf(state: StackState, id: string): Step | undefined {
  return state.steps.find((s) => s.step_id === id) ?? state.excluded[id];
}

/** A change to the stack in words: "removing gain", "changing high_pass". */
export function changeWords(state: StackState, c: StackChange): string {
  const ops = c.step_ids.map((id) => stepOf(state, id)?.op ?? "?").join(", ");
  const verb = ({ applied: "applying", excluded: "removing", restored: "restoring" } as Record<string, string>)[c.kind] ?? "changing";
  return `${verb} ${ops}`;
}

/** What can be done to the stack; absent when the project cannot be changed. */
export interface StackActions {
  remove: (id: string, reason?: string) => void;
  restore: (id: string) => void;
  edit: (id: string, changes: Record<string, unknown>) => void;
  remeasure: (id: string) => void;
  remeasureDiff: (id: string) => Promise<DiffEntry[]>;
  undo: () => void;
  redo: () => void;
}

/**
 * The stack, every step in its place. A removed step stays where it was,
 * greyed, and can be restored; the steps above a change keep their values.
 * Clicking a step selects it: its values can be changed, and the monitor can
 * play the audio before it, after it, and what it removed.
 */
export function StackPanel({
  state,
  onClone,
  canClone,
  brokenAtLine,
  actions,
  busy,
  selected,
  onSelect,
  descriptors,
  onRate,
  output,
}: {
  state: StackState;
  onClone: (atStep?: string) => void;
  canClone: boolean;
  /** When the log did not verify: the stack shows only what was recorded before this line. */
  brokenAtLine: number | null;
  actions?: StackActions;
  busy: boolean;
  /** The selected step, if any. */
  selected: string | null;
  onSelect: (id: string | null) => void;
  descriptors: Descriptor[];
  /** Rate the stack as it stands, 1–5. */
  onRate?: (overall: number) => void;
  /** With time edits: the output's length and the original's (seconds). */
  output?: { duration: number; original: number } | null;
}) {
  // The step whose removal is being confirmed (with an optional reason).
  const [removing, setRemoving] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const lastUndo = state.undo[state.undo.length - 1];
  const lastRedo = state.redo[state.redo.length - 1];
  const removed = state.entries.length - state.steps.length;
  return (
    <div className="panel" data-testid="stack">
      <div className="panel-head">
        <h3>Stack</h3>
        {actions && (
          <>
            <button
              className="small"
              disabled={busy || !lastUndo}
              onClick={actions.undo}
              data-testid="undo"
              title={lastUndo ? `Undo ${changeWords(state, lastUndo)} (it stays in the log)` : "Nothing to undo"}
            >
              Undo
            </button>
            <button
              className="small"
              disabled={busy || !lastRedo}
              onClick={actions.redo}
              data-testid="redo"
              title={lastRedo ? `Redo ${changeWords(state, lastRedo)}` : "Nothing to redo"}
            >
              Redo
            </button>
          </>
        )}
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
      {state.entries.length === 0 && brokenAtLine === null && <div className="muted">No steps yet. The original is untouched.</div>}
      {output && (
        <div className="output-length" data-testid="output-length">
          Output {output.duration.toFixed(3)} s (original {output.original.toFixed(3)} s). Removed stretches are shaded on the lanes and
          skipped when playing <i>Processed</i>.
        </div>
      )}
      {onRate && state.steps.length > 0 && (
        <div className="stack-actions">
          <span className="rate" title="How good is the result? Recorded for learning.">
            Rate:
            {[1, 2, 3, 4, 5].map((n) => (
              <button key={n} className="small" onClick={() => onRate(n)} data-testid={`rate-${n}`}>
                {n}
              </button>
            ))}
          </span>
        </div>
      )}
      <ol className="steps">
        {state.entries.map((e) => {
          const s = stepOf(state, e.step_id);
          if (!s) return null;
          if (!e.active)
            return (
              <li key={s.step_id} className="removed" data-testid="stack-step-removed" data-op={s.op} data-step={s.step_id}>
                <div className="step-top">
                  <span className="op">{s.op}</span>
                  <span className="label">removed{e.reason ? `: ${e.reason}` : ""}</span>
                  {actions && (
                    <button
                      className="small"
                      disabled={busy}
                      onClick={() => actions.restore(s.step_id)}
                      data-testid="restore-step"
                      title="Bring it back to its place, with the values it had"
                    >
                      Restore
                    </button>
                  )}
                </div>
                <div className="step-desc">{describe(s)}</div>
              </li>
            );
          const isSelected = selected === s.step_id;
          return (
            <li
              key={s.step_id}
              className={isSelected ? "selected" : ""}
              data-testid="stack-step"
              data-op={s.op}
              data-step={s.step_id}
              aria-selected={isSelected}
            >
              <div className="step-top">
                <button
                  className="op link"
                  onClick={() => onSelect(isSelected ? null : s.step_id)}
                  data-testid="select-step"
                  title={isSelected ? "Close" : "Change its values, and listen to it on its own"}
                >
                  {s.op}
                </button>
                {s.plan_id && <span className="tag">plan</span>}
                {s.inherited_from && <span className="tag">inherited</span>}
                {e.edits > 0 && <span className="tag">edited</span>}
                {e.drifted && (
                  <button
                    className="tag drift"
                    onClick={() => onSelect(s.step_id)}
                    data-testid="drifted"
                    title="Its values were measured on audio that has changed since: a step below it was removed, restored or changed"
                  >
                    measured before a change
                  </button>
                )}
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
                {actions && (
                  <button
                    className="small"
                    disabled={busy}
                    onClick={() => {
                      setRemoving(removing === s.step_id ? null : s.step_id);
                      setReason("");
                    }}
                    data-testid="remove-step"
                    title="Remove it: it keeps its place and can be restored; the steps above keep their values"
                  >
                    ×
                  </button>
                )}
              </div>
              <div className="step-desc">{describe(s)}</div>
              <div className="step-meta" data-testid="step-meta">
                {s.actor.model ? `${s.actor.model}${s.actor.provider ? ` (${s.actor.provider})` : ""}` : s.actor.kind} via {s.origin}
                {s.intent ? ` — “${s.intent}”` : ""}
              </div>
              {actions && removing === s.step_id && (
                <div className="editor-actions">
                  <input
                    placeholder="Why? (optional, recorded)"
                    value={reason}
                    onChange={(ev) => setReason(ev.target.value)}
                    data-testid="remove-reason"
                  />
                  <button
                    disabled={busy}
                    onClick={() => {
                      setRemoving(null);
                      actions.remove(s.step_id, reason.trim() || undefined);
                    }}
                    data-testid="remove-confirm"
                  >
                    Remove
                  </button>
                </div>
              )}
              {actions && isSelected && (
                <StepEditor
                  step={s}
                  descriptor={descriptors.find((d) => d.id === s.op && d.version === s.op_version)}
                  drifted={e.drifted}
                  busy={busy}
                  onApply={(changes) => actions.edit(s.step_id, changes)}
                  onRemeasure={() => actions.remeasure(s.step_id)}
                  loadDiff={() => actions.remeasureDiff(s.step_id)}
                />
              )}
            </li>
          );
        })}
      </ol>
      {state.steps.length > 0 && (
        <div className="muted small-text" data-testid="approval-note">
          Exporting approves the stack as it stands: {state.steps.length} step{state.steps.length === 1 ? "" : "s"}
          {removed > 0 ? `; ${removed} removed, left out` : ""}.
        </div>
      )}
    </div>
  );
}
