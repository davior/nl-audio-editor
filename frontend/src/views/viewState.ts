// Interface state: what part of the recording is on screen and how it is drawn.
// It is stored in the project's manifest so a project reopens as it was left.
// It is not evidence and is never logged.
import type { ViewState, Which } from "../core/types";

/** Shortest span the lanes zoom to, in seconds. */
export const MIN_SPAN = 0.02;

export const DB_RANGES = [30, 45, 60, 75, 90, 120, 150];

export function defaultView(duration: number, sampleRate: number): ViewState {
  return {
    t0: 0,
    t1: duration,
    scale: "linear",
    fMax: Math.min(8000, sampleRate / 2),
    dbRange: 90,
    vZoom: 1,
    monitor: "source",
    selection: null,
    tfSelection: null,
  };
}

const num = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

/** The saved view, checked field by field; anything missing or out of range falls back to the default. */
export function restoreView(saved: Record<string, unknown> | undefined, duration: number, sampleRate: number): ViewState {
  const d = defaultView(duration, sampleRate);
  if (!saved) return d;
  const v: ViewState = { ...d };
  if (num(saved.t0) && num(saved.t1) && saved.t0 >= 0 && saved.t1 <= duration + 1e-9 && saved.t1 - saved.t0 >= MIN_SPAN / 2) {
    v.t0 = saved.t0;
    v.t1 = saved.t1;
  }
  if (saved.scale === "linear" || saved.scale === "log") v.scale = saved.scale;
  if (num(saved.fMax) && saved.fMax > 100 && saved.fMax <= sampleRate / 2) v.fMax = saved.fMax;
  if (num(saved.dbRange) && saved.dbRange >= 10 && saved.dbRange <= 200) v.dbRange = saved.dbRange;
  if (num(saved.vZoom) && saved.vZoom >= 1 && saved.vZoom <= 1e6) v.vZoom = saved.vZoom;
  if (saved.monitor === "source" || saved.monitor === "stack" || saved.monitor === "residual") v.monitor = saved.monitor as Which;
  const sel = saved.selection as Record<string, unknown> | null | undefined;
  if (sel && num(sel.t0) && num(sel.t1) && sel.t1 > sel.t0) v.selection = { t0: sel.t0, t1: sel.t1 };
  const tf = saved.tfSelection as Record<string, unknown> | null | undefined;
  if (tf && num(tf.t0) && num(tf.t1) && num(tf.f_lo) && num(tf.f_hi) && tf.t1 > tf.t0 && tf.f_hi > tf.f_lo) {
    v.tfSelection = { t0: tf.t0, t1: tf.t1, f_lo: tf.f_lo, f_hi: tf.f_hi };
  }
  return v;
}

function place(v: ViewState, t0: number, span: number, duration: number): ViewState {
  const s = Math.min(Math.max(span, Math.min(MIN_SPAN, duration)), duration);
  const a = Math.min(Math.max(0, t0), Math.max(0, duration - s));
  return { ...v, t0: a, t1: a + s };
}

/** Zoom by `factor` (> 1 zooms out), keeping the time under the pointer where it is. */
export function zoom(v: ViewState, factor: number, around: number, duration: number): ViewState {
  const span = v.t1 - v.t0;
  const next = Math.min(Math.max(span * factor, MIN_SPAN), duration);
  const u = span > 0 ? (around - v.t0) / span : 0.5;
  return place(v, around - u * next, next, duration);
}

export function scroll(v: ViewState, dt: number, duration: number): ViewState {
  return place(v, v.t0 + dt, v.t1 - v.t0, duration);
}

export function fit(v: ViewState, duration: number): ViewState {
  return { ...v, t0: 0, t1: duration };
}

export function showRange(v: ViewState, t0: number, t1: number, duration: number): ViewState {
  const pad = (t1 - t0) * 0.05;
  return place(v, t0 - pad, t1 - t0 + 2 * pad, duration);
}

/** Page the view forward when the playhead runs off the right edge. */
export function follow(v: ViewState, t: number, duration: number): ViewState {
  const span = v.t1 - v.t0;
  if (t < v.t1 || span >= duration) return v;
  return place(v, t, span, duration);
}
