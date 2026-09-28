// One step of the stack, in its place under the request that made it. A
// removed step stays where it was, greyed, and can be restored; the steps
// above a change keep their values. Clicking a step's name selects it: its
// values can be changed, and the monitor can play the audio before it, after
// it, and what it removed.
import { useState } from "react";
import type { DiffEntry, StackChange, StackEntry, StackState, Step } from "../core/types";
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
    case "pitch_shift":
      return `${(r.semitones as number) > 0 ? "+" : ""}${n("semitones", 2)} semitones (×${n("ratio", 4)}), formants ${r.preserve_formants === false ? "moved" : "kept"}`;
    default:
      return "";
  }
}

export function scopeWords(s: Step["scope"]): string {
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

export function StepRow({
  n,
  step: s,
  entry: e,
  actions,
  busy,
  selected,
  onSelect,
  descriptors,
  onClone,
  canClone,
  meta,
}: {
  /** Its place in the stack, from 1. */
  n: number;
  step: Step;
  entry: StackEntry;
  actions?: StackActions;
  busy: boolean;
  selected: boolean;
  onSelect: (id: string | null) => void;
  descriptors: Descriptor[];
  onClone: (atStep: string) => void;
  canClone: boolean;
  /** Say who made it and from what words (when no request above says so). */
  meta?: boolean;
}) {
  // Removing it is confirmed, with an optional reason.
  const [removing, setRemoving] = useState(false);
  const [reason, setReason] = useState("");
  const measured = measurementText(s.measurements as Record<string, unknown>);
  const who = meta && (
    <div className="step-meta" data-testid="step-meta">
      {s.actor.model ? `${s.actor.model}${s.actor.provider ? ` (${s.actor.provider})` : ""}` : s.actor.kind} via {s.origin}
      {s.intent ? ` — “${s.intent}”` : ""}
    </div>
  );
  if (!e.active)
    return (
      <li value={n} className="removed" data-testid="stack-step-removed" data-op={s.op} data-step={s.step_id}>
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
        {who}
      </li>
    );
  return (
    <li
      value={n}
      className={selected ? "selected" : ""}
      data-testid="stack-step"
      data-op={s.op}
      data-step={s.step_id}
      aria-selected={selected}
    >
      <div className="step-top">
        <button
          className="op link"
          onClick={() => onSelect(selected ? null : s.step_id)}
          data-testid="select-step"
          title={selected ? "Close" : "Change its values, and listen to it on its own"}
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
        <span className="label">
          {scopeWords(s.scope)} · {s.label}
        </span>
        {canClone && (
          <button className="small" onClick={() => onClone(s.step_id)} title="Clone the project at this step" data-testid="clone-here">
            Clone here
          </button>
        )}
        {actions && (
          <button
            className="small"
            disabled={busy}
            onClick={() => {
              setRemoving(!removing);
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
      {measured && (
        <div className="step-meta" data-testid="measurements">
          {measured}
        </div>
      )}
      {who}
      {actions && removing && (
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
              setRemoving(false);
              actions.remove(s.step_id, reason.trim() || undefined);
            }}
            data-testid="remove-confirm"
          >
            Remove
          </button>
        </div>
      )}
      {actions && selected && (
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
}
