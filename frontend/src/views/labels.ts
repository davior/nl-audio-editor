// Labels: short notes on a stretch or an area of the recording. They are listed beside the
// stack and are not drawn on the lanes; clicking one selects its place again. The list is
// rebuilt from the log by the core; this is only how the page orders, words and reselects them.
import type { Label, ViewState } from "../core/types";
import { fMaxOptions, reveal } from "./viewState";

/** The longest a label's text may be: one line (the core enforces the same limit). */
export const MAX_LABEL_CHARS = 200;

/** What a label marks: a time range, or, with a band, a time × frequency area. */
export interface Place {
  t0: number;
  t1: number;
  f_lo?: number;
  f_hi?: number;
}

/** Which selection a label is made from: the waveform's time range or the spectrogram's area. */
export type Kind = "range" | "area";

export const isArea = (p: { f_lo?: number | null; f_hi?: number | null }): boolean => p.f_lo != null && p.f_hi != null;

/** The text cut short for a line of its own: at most `n` characters, then an ellipsis. */
export function brief(text: string, n = 40): string {
  const chars = Array.from(text);
  return chars.length > n ? `${chars.slice(0, n - 1).join("")}…` : text;
}

/** "1.250–1.750 s", or with a band "1.000–2.000 s × 300–900 Hz". */
export function placeWords(p: { t0: number; t1: number; f_lo?: number | null; f_hi?: number | null }): string {
  const t = `${p.t0.toFixed(3)}–${p.t1.toFixed(3)} s`;
  return p.f_lo != null && p.f_hi != null ? `${t} × ${Math.round(p.f_lo)}–${Math.round(p.f_hi)} Hz` : t;
}

/** In time order, as along the recording; labels of the same place keep the order they were added in. */
export function inTimeOrder(labels: Label[]): Label[] {
  return [...labels].sort((a, b) => a.t0 - b.t0 || a.t1 - b.t1 || a.seq - b.seq);
}

const within = (x: number, lo: number, hi: number) => Math.min(Math.max(x, lo), hi);

/**
 * The place the view's selection marks, kept inside the recording; null when that selection is not
 * there. (A drag cannot leave the lane, so this only guards against rounding at the very edges.)
 */
export function placeOf(kind: Kind, view: ViewState, duration: number, sampleRate: number): Place | null {
  const s = kind === "range" ? view.selection : view.tfSelection;
  if (!s) return null;
  const t0 = within(s.t0, 0, duration);
  const t1 = within(s.t1, 0, duration);
  if (!(t1 > t0)) return null;
  if (kind === "range") return { t0, t1 };
  const a = view.tfSelection!;
  const f_lo = within(a.f_lo, 0, sampleRate / 2);
  const f_hi = within(a.f_hi, 0, sampleRate / 2);
  return f_hi > f_lo ? { t0, t1, f_lo, f_hi } : null;
}

/** The label's place is what is selected now. */
export function isSelected(l: Label, view: ViewState): boolean {
  if (isArea(l)) {
    const s = view.tfSelection;
    return !!s && s.t0 === l.t0 && s.t1 === l.t1 && s.f_lo === l.f_lo && s.f_hi === l.f_hi;
  }
  const s = view.selection;
  return !!s && s.t0 === l.t0 && s.t1 === l.t1;
}

/**
 * The view with the label's place selected, and no other selection, and in sight: the time window
 * moves to hold it, and the spectrogram's top frequency rises when an area reaches above it.
 */
export function showLabel(v: ViewState, l: Label, duration: number, sampleRate: number): ViewState {
  const seen = reveal(v, l.t0, l.t1, duration);
  if (l.f_lo == null || l.f_hi == null) return { ...seen, selection: { t0: l.t0, t1: l.t1 }, tfSelection: null };
  const fMax = l.f_hi > seen.fMax ? (fMaxOptions(sampleRate).find((f) => f >= l.f_hi!) ?? sampleRate / 2) : seen.fMax;
  return { ...seen, fMax, selection: null, tfSelection: { t0: l.t0, t1: l.t1, f_lo: l.f_lo, f_hi: l.f_hi } };
}
