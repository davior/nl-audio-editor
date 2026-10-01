// Labels: a list beside the stack, and the small form that adds one. A label is a one-line note
// on a stretch (or an area) of the recording. Nothing is drawn on the lanes: clicking a row
// selects its place again. The list is the log's, so it is the same when the project is opened
// again. Changing the text is a click on ✎; × takes a label off the list (its words stay in the
// log). The place of a label cannot be changed: remove it and make another.
import { useEffect, useRef, useState } from "react";
import type { Label, ViewState } from "../core/types";
import { inTimeOrder, isSelected, MAX_LABEL_CHARS, placeWords, type Kind, type Place } from "./labels";

/** The text of a label being changed: Enter or leaving the box keeps it, Escape does not. */
function EditText({ initial, onSave, onCancel }: { initial: string; onSave: (text: string) => void; onCancel: () => void }) {
  const [text, setText] = useState(initial);
  const input = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const finish = (how: "enter" | "escape" | "blur") => {
    if (done.current) return;
    const t = text.trim();
    if (how === "enter" && !t) return; // an empty label is not a label: keep editing
    done.current = true;
    if (how !== "escape" && t && t !== initial) onSave(t);
    else onCancel();
  };
  return (
    <input
      ref={input}
      className="label-edit"
      value={text}
      maxLength={MAX_LABEL_CHARS}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => finish("blur")}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          finish("enter");
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish("escape");
        }
      }}
      title="Enter keeps the change, Escape drops it"
      aria-label="Label text"
      data-testid="label-edit-input"
    />
  );
}

function LabelRow({
  label,
  selected,
  editing,
  editable,
  why,
  onSelect,
  onStartEdit,
  onSave,
  onStopEdit,
  onRemove,
}: {
  label: Label;
  selected: boolean;
  editing: boolean;
  editable: boolean;
  why: string | null;
  onSelect: () => void;
  onStartEdit: () => void;
  onSave: (text: string) => void;
  onStopEdit: () => void;
  onRemove: () => void;
}) {
  const where = placeWords(label);
  const off = editable ? undefined : (why ?? undefined);
  return (
    <li className={`label-row${selected ? " selected" : ""}`} data-testid="label-row" data-label-id={label.id}>
      {editing ? (
        <div className="label-main">
          <span className="label-span mono">{where}</span>
          <EditText initial={label.text} onSave={onSave} onCancel={onStopEdit} />
        </div>
      ) : (
        <>
          <button
            className="label-main"
            onClick={onSelect}
            title={`Select ${where} again`}
            aria-pressed={selected}
            data-testid="label-select"
          >
            <span className="label-span mono" data-testid="label-place">
              {where}
            </span>
            <span className="label-text" data-testid="label-text">
              {label.text}
            </span>
          </button>
          <button
            className="small"
            onClick={onStartEdit}
            disabled={!editable}
            title={off ?? "Change the text"}
            aria-label={`Edit label “${label.text}”`}
            data-testid="label-edit"
          >
            ✎
          </button>
          <button
            className="small"
            onClick={onRemove}
            disabled={!editable}
            title={off ?? "Delete this label (its words stay in the log)"}
            aria-label={`Delete label “${label.text}”`}
            data-testid="label-delete"
          >
            ×
          </button>
        </>
      )}
    </li>
  );
}

export function LabelsPanel({
  labels,
  view,
  editable,
  why,
  scrollTo,
  onSelect,
  onEdit,
  onRemove,
}: {
  labels: Label[];
  view: ViewState;
  /** Labels can be changed and removed (not in a project that did not verify). */
  editable: boolean;
  /** Why not, when they cannot. */
  why: string | null;
  /** A label to bring into sight in the list, such as the one just added (each new `n` once). */
  scrollTo: { id: string; n: number } | null;
  onSelect: (l: Label) => void;
  onEdit: (id: string, text: string) => void;
  onRemove: (id: string) => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  const list = useRef<HTMLOListElement>(null);
  const rows = inTimeOrder(labels);

  // Only the list scrolls, never the page: the label just added may be anywhere in it. Once per
  // label added, as soon as its row is there (a later change to the list leaves the scroll alone).
  const scrolled = useRef(0);
  useEffect(() => {
    const ol = list.current;
    const row = scrollTo ? ol?.querySelector<HTMLElement>(`[data-label-id="${scrollTo.id}"]`) : null;
    if (!scrollTo || !ol || !row || scrolled.current === scrollTo.n) return;
    scrolled.current = scrollTo.n;
    if (row.offsetTop < ol.scrollTop) ol.scrollTop = row.offsetTop;
    else if (row.offsetTop + row.offsetHeight > ol.scrollTop + ol.clientHeight)
      ol.scrollTop = row.offsetTop + row.offsetHeight - ol.clientHeight;
  }, [scrollTo, labels.length]);

  return (
    <div className="panel labels" data-testid="labels">
      <div className="panel-head">
        <h3>Labels</h3>
        <span className="muted small-text" data-testid="label-count">
          {labels.length === 0 ? "" : labels.length}
        </span>
      </div>
      {rows.length === 0 ? (
        <p className="muted small-text" data-testid="labels-empty">
          No labels yet. Select a stretch on the waveform, or an area on the spectrogram, then right-click it (or press ＋ Label) to note
          what is there. Labels are not drawn on the recording: click one here to select its place again.
        </p>
      ) : (
        <ol className="labels-list" ref={list} data-testid="labels-list">
          {rows.map((l) => (
            <LabelRow
              key={l.id}
              label={l}
              selected={isSelected(l, view)}
              editing={editing === l.id}
              editable={editable}
              why={why}
              onSelect={() => onSelect(l)}
              onStartEdit={() => setEditing(l.id)}
              onSave={(text) => {
                setEditing(null);
                onEdit(l.id, text);
              }}
              onStopEdit={() => setEditing(null)}
              onRemove={() => {
                setEditing(null);
                onRemove(l.id);
              }}
            />
          ))}
        </ol>
      )}
    </div>
  );
}

/** Room the popover needs, to keep it on screen (CSS gives it the same width). */
const POPOVER = { w: 300, h: 110 };

/**
 * The form that adds a label to what is selected, where it was asked for (the lane that was
 * right-clicked, or the ＋ Label button). It closes when anything else is clicked, or on Escape.
 */
export function AddLabel({
  kind,
  place,
  at,
  editable,
  why,
  onAdd,
  onClose,
}: {
  kind: Kind;
  /** What the label would mark; null when that selection is not there. */
  place: Place | null;
  /** Where on the screen, in client coordinates. */
  at: { x: number; y: number };
  editable: boolean;
  why: string | null;
  onAdd: (text: string) => void;
  onClose: () => void;
}) {
  const [text, setText] = useState("");
  const box = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    input.current?.focus();
    const away = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) close.current();
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") close.current();
    };
    const leave = () => close.current();
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", escape);
    window.addEventListener("blur", leave);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", escape);
      window.removeEventListener("blur", leave);
    };
  }, []);

  const left = Math.max(8, Math.min(at.x, window.innerWidth - POPOVER.w - 8));
  const top = Math.max(8, Math.min(at.y, window.innerHeight - POPOVER.h - 8));
  const hint = !editable
    ? why
    : kind === "range"
      ? "Drag on the waveform to select a stretch, then right-click it to label it."
      : "Drag on the spectrogram to select an area, then right-click it to label it.";

  return (
    <div ref={box} className="label-popover" style={{ left, top }} role="dialog" aria-label="Add a label" data-testid="label-popover">
      {place && editable ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const t = text.trim();
            if (t) onAdd(t);
          }}
        >
          <div className="where" data-testid="label-popover-place">
            Label {placeWords(place)}
          </div>
          <div className="row">
            <input
              ref={input}
              value={text}
              maxLength={MAX_LABEL_CHARS}
              onChange={(e) => setText(e.target.value)}
              placeholder="What is here?"
              aria-label="Label text"
              data-testid="label-input"
            />
            <button type="submit" disabled={!text.trim()} title="Add the label (Enter)" data-testid="label-add">
              ＋ Add
            </button>
          </div>
        </form>
      ) : (
        <div className="where" data-testid="label-popover-hint">
          {hint}
        </div>
      )}
    </div>
  );
}
