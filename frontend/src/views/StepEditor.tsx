// A stack step's values, in a form generated from its operation's descriptor
// (the registry's limits included). Apply sends only the values that changed:
// the core measures the step again on its input and refuses a value out of
// range; it never clamps one. A step measured on audio that has changed since
// can be measured again, with what that would change shown first.
import { useEffect, useRef, useState } from "react";
import { core } from "../core/client";
import type { DiffEntry, Step } from "../core/types";

export type ParamSpec = {
  id: string;
  title: string;
  type: string;
  min?: number;
  max?: number;
  step?: number;
  values?: string[];
  unit?: string;
};

export type Descriptor = { id: string; version: number; params: ParamSpec[] };

let loading: Promise<Descriptor[]> | null = null;

/** The registry's descriptors, loaded once for the page. */
export function useDescriptors(onError: (e: unknown) => void): Descriptor[] {
  const [descriptors, setDescriptors] = useState<Descriptor[]>([]);
  useEffect(() => {
    loading ??= (core.descriptors() as Promise<Descriptor[]>).catch((e) => {
      loading = null;
      throw e;
    });
    let live = true;
    loading.then((d) => {
      if (live) setDescriptors(d);
    }, onError);
    return () => {
      live = false;
    };
  }, [onError]);
  return descriptors;
}

/** A value editor for one parameter; numbers, choices and switches only. */
export function ParamInput({ spec, value, onChange }: { spec: ParamSpec; value: unknown; onChange: (v: unknown) => void }) {
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

const EDITABLE = ["number", "integer", "enum", "bool"];

function short(v: unknown): string {
  if (typeof v === "number") return v.toFixed(2);
  if (Array.isArray(v)) return `${v.length} values`;
  if (v === null || v === undefined) return "—";
  return String(v).slice(0, 30);
}

export function StepEditor({
  step,
  descriptor,
  drifted,
  busy,
  onApply,
  onRemeasure,
  loadDiff,
}: {
  step: Step;
  descriptor: Descriptor | undefined;
  /** Measured on audio that has changed since (a step below was removed, restored or edited). */
  drifted: boolean;
  busy: boolean;
  onApply: (changes: Record<string, unknown>) => void;
  onRemeasure: () => void;
  /** What measuring the step again would change (nothing is changed). */
  loadDiff: () => Promise<DiffEntry[]>;
}) {
  const [changes, setChanges] = useState<Record<string, unknown>>({});
  const [diff, setDiff] = useState<DiffEntry[] | null>(null);
  const params = step.params as Record<string, unknown>;
  // The page redraws often (the playhead); the diff is asked for once per version.
  const load = useRef(loadDiff);
  load.current = loadDiff;

  // A new version of the step (applied, or changed elsewhere) starts afresh.
  useEffect(() => {
    setChanges({});
    setDiff(null);
  }, [step.step_id, step.stack_hash]);

  useEffect(() => {
    if (!drifted) return;
    let live = true;
    load.current().then(
      (d) => {
        if (live) setDiff(d);
      },
      () => {
        if (live) setDiff([]);
      },
    );
    return () => {
      live = false;
    };
  }, [drifted, step.stack_hash]);

  const specs = descriptor?.params.filter((p) => EDITABLE.includes(p.type)) ?? [];
  const changed = Object.keys(changes).length > 0;
  return (
    <div className="step-editor" data-testid="step-editor" data-step={step.step_id}>
      {specs.length === 0 ? (
        <div className="muted small-text">This operation has no values to change here.</div>
      ) : (
        <div className="param-editor">
          {specs.map((p) => (
            <label key={p.id}>
              {p.title}
              {p.unit ? ` (${p.unit})` : ""}
              <ParamInput
                spec={p}
                value={p.id in changes ? changes[p.id] : params[p.id]}
                onChange={(v) => setChanges((c) => ({ ...c, [p.id]: v }))}
              />
            </label>
          ))}
        </div>
      )}
      {specs.length > 0 && (
        <div className="editor-actions">
          <button
            className="accept"
            disabled={busy || !changed}
            onClick={() => onApply(changes)}
            data-testid="apply-edit"
            title="Measure the step again with these values; the steps above keep theirs"
          >
            Apply
          </button>
          <button disabled={busy || !changed} onClick={() => setChanges({})} data-testid="reset-edit">
            Reset
          </button>
        </div>
      )}
      {drifted && (
        <div className="drift-note" data-testid="drift-note">
          Its values were measured on audio that has changed since. Measuring it again would change
          {diff === null ? (
            " …"
          ) : diff.length === 0 ? (
            " nothing."
          ) : (
            <ul data-testid="remeasure-diff">
              {diff.map((d) => (
                <li key={d.param}>
                  {d.param.replace(/^resolved\./, "")}: {short(d.recorded)} → {short(d.new)}
                </li>
              ))}
            </ul>
          )}
          <button disabled={busy} onClick={onRemeasure} data-testid="remeasure">
            Measure again
          </button>
        </div>
      )}
    </div>
  );
}
