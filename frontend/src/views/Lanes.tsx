// The waveform and spectrogram lanes. They draw what the core computes; all
// audio work happens in the worker.
import { useEffect, useRef, useState } from "react";
import { core } from "../core/client";
import type { ViewState, Which } from "../core/types";
import { debug } from "../debug";
import { INFERNO } from "./colormap";

interface LaneProps {
  projectId: string;
  which: Which;
  view: ViewState;
  width: number;
  duration: number;
  sampleRate: number;
  playhead: number;
  onSeek: (t: number) => void;
  onSelect: (sel: ViewState["selection"]) => void;
  onSelectTf: (sel: ViewState["tfSelection"]) => void;
  onZoom: (factor: number, around: number) => void;
  onScroll: (dt: number) => void;
  /** Changes whenever what the lanes draw changes (the stack hash). */
  renderKey: string;
  /** Called when a spectrogram view has fully arrived. */
  onComplete?: () => void;
}

function timeAt(x: number, width: number, v: ViewState) {
  return v.t0 + (x / width) * (v.t1 - v.t0);
}

function xAt(t: number, width: number, v: ViewState) {
  return ((t - v.t0) / (v.t1 - v.t0)) * width;
}

function freqAt(y: number, height: number, v: ViewState, fMin: number) {
  const u = 1 - y / height;
  if (v.scale === "log") {
    const l0 = Math.log2(fMin);
    const l1 = Math.log2(v.fMax);
    return Math.pow(2, l0 + u * (l1 - l0));
  }
  return u * v.fMax;
}

function yAt(f: number, height: number, v: ViewState, fMin: number) {
  if (v.scale === "log") {
    const l0 = Math.log2(fMin);
    const l1 = Math.log2(v.fMax);
    return (1 - (Math.log2(Math.max(f, fMin)) - l0) / (l1 - l0)) * height;
  }
  return (1 - f / v.fMax) * height;
}

// Wheel zooms around the pointer; shift+wheel scrolls. Registered as a
// non-passive listener so the page itself does not scroll.
function useWheel(ref: React.RefObject<HTMLCanvasElement | null>, p: LaneProps) {
  const latest = useRef(p);
  latest.current = p;
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const q = latest.current;
      const r = c.getBoundingClientRect();
      const t = timeAt(e.clientX - r.left, q.width, q.view);
      if (e.shiftKey) q.onScroll((e.deltaY / 1000) * (q.view.t1 - q.view.t0));
      else q.onZoom(e.deltaY > 0 ? 1.25 : 0.8, t);
    };
    c.addEventListener("wheel", onWheel, { passive: false });
    return () => c.removeEventListener("wheel", onWheel);
  }, [ref]);
}

function useDrag(onClick: (x: number, y: number) => void, onDrag: (x0: number, y0: number, x1: number, y1: number, done: boolean) => void) {
  const start = useRef<{ x: number; y: number } | null>(null);
  return {
    onMouseDown: (e: React.MouseEvent<HTMLCanvasElement>) => {
      const r = e.currentTarget.getBoundingClientRect();
      start.current = { x: e.clientX - r.left, y: e.clientY - r.top };
    },
    onMouseMove: (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (!start.current) return;
      const r = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - r.left;
      const y = e.clientY - r.top;
      if (Math.abs(x - start.current.x) > 3 || Math.abs(y - start.current.y) > 3) onDrag(start.current.x, start.current.y, x, y, false);
    },
    onMouseUp: (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (!start.current) return;
      const r = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - r.left;
      const y = e.clientY - r.top;
      if (Math.abs(x - start.current.x) <= 3 && Math.abs(y - start.current.y) <= 3) onClick(x, y);
      else onDrag(start.current.x, start.current.y, x, y, true);
      start.current = null;
    },
    onMouseLeave: () => {
      start.current = null;
    },
  };
}

/** What a lane last received, with the request it answers. */
interface LaneData<T> {
  which: Which;
  t0: number;
  t1: number;
  columns: number;
  values: T;
}

/** Tracks requests in flight so a lane can say it is still computing. */
function usePending(): [boolean, () => () => void] {
  const [count, setCount] = useState(0);
  const start = () => {
    setCount((c) => c + 1);
    let done = false;
    return () => {
      if (done) return;
      done = true;
      setCount((c) => c - 1);
    };
  };
  return [count > 0, start];
}

function Busy({ on, id }: { on: boolean; id: string }) {
  return on ? (
    <div className="lane-busy" data-testid={id}>
      computing…
    </div>
  ) : null;
}

export function Waveform(p: LaneProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [peaks, setPeaks] = useState<LaneData<Float32Array> | null>(null);
  const [drag, setDrag] = useState<[number, number] | null>(null);
  const [pending, begin] = usePending();
  const height = 140;
  useWheel(ref, p);

  useEffect(() => {
    let live = true;
    if (p.width <= 0) return;
    const req = { which: p.which, t0: p.view.t0, t1: p.view.t1, columns: p.width };
    const end = begin();
    core
      .peaks(p.projectId, req.which, req.t0, req.t1, req.columns)
      .then((values) => live && setPeaks({ ...req, values }))
      .finally(end);
    return () => {
      live = false;
    };
  }, [p.projectId, p.which, p.view.t0, p.view.t1, p.width, p.renderKey]);

  useEffect(() => {
    const c = ref.current;
    if (!c || !peaks) return;
    const g = c.getContext("2d")!;
    g.fillStyle = "#101418";
    g.fillRect(0, 0, c.width, c.height);
    const mid = height / 2;
    g.strokeStyle = "#2a3440";
    g.beginPath();
    g.moveTo(0, mid);
    g.lineTo(c.width, mid);
    g.stroke();
    g.strokeStyle = "#59c2ff";
    g.beginPath();
    let drawn = 0;
    // Until new peaks arrive after a zoom or scroll, the last ones are drawn where they now fall.
    const span = peaks.t1 - peaks.t0;
    for (let x = 0; x < p.width; x++) {
      const i = Math.floor(((timeAt(x + 0.5, p.width, p.view) - peaks.t0) / span) * peaks.columns);
      if (i < 0 || i >= peaks.columns) continue;
      const lo = peaks.values[2 * i] * p.view.vZoom;
      const hi = peaks.values[2 * i + 1] * p.view.vZoom;
      const y0 = mid - Math.max(-1, Math.min(1, hi)) * (mid - 2);
      const y1 = mid - Math.max(-1, Math.min(1, lo)) * (mid - 2);
      g.moveTo(x + 0.5, y0);
      g.lineTo(x + 0.5, Math.max(y1, y0 + 1));
      if (hi - lo > 1e-6) drawn++;
    }
    g.stroke();
    const sel = drag
      ? { t0: timeAt(Math.min(...drag), p.width, p.view), t1: timeAt(Math.max(...drag), p.width, p.view) }
      : p.view.selection;
    if (sel) {
      const a = xAt(sel.t0, p.width, p.view);
      const b = xAt(sel.t1, p.width, p.view);
      g.fillStyle = "rgba(255, 214, 102, 0.18)";
      g.fillRect(a, 0, b - a, height);
      g.strokeStyle = "rgba(255, 214, 102, 0.8)";
      g.strokeRect(a + 0.5, 0.5, b - a - 1, height - 1);
    }
    const px = xAt(p.playhead, p.width, p.view);
    g.strokeStyle = "#ff5c5c";
    g.beginPath();
    g.moveTo(px + 0.5, 0);
    g.lineTo(px + 0.5, height);
    g.stroke();
    debug("waveform", { columns: p.width, columnsWithSignal: drawn, which: peaks.which, t0: peaks.t0, t1: peaks.t1 });
  }, [peaks, p.view, p.playhead, p.width, drag]);

  const handlers = useDrag(
    (x) => p.onSeek(timeAt(x, p.width, p.view)),
    (x0, _y0, x1, _y1, done) => {
      if (done) {
        setDrag(null);
        p.onSelect({ t0: timeAt(Math.min(x0, x1), p.width, p.view), t1: timeAt(Math.max(x0, x1), p.width, p.view) });
      } else setDrag([x0, x1]);
    },
  );

  return (
    <div className="lane-wrap">
      <canvas ref={ref} width={p.width} height={height} className="lane" data-testid="waveform" {...handlers} />
      <Busy on={pending} id="waveform-busy" />
    </div>
  );
}

interface SpecData extends LaneData<Uint8Array> {
  rows: number;
  fMax: number;
  scale: ViewState["scale"];
  /** All columns have arrived (long views arrive in parts, left to right). */
  complete: boolean;
}

/** Finished spectrogram levels by project, render and request, most recent last. */
const specCache = new Map<string, Uint8Array>();
const SPEC_CACHE_ENTRIES = 8;

function cacheGet(key: string): Uint8Array | undefined {
  const v = specCache.get(key);
  if (v) {
    specCache.delete(key);
    specCache.set(key, v);
  }
  return v;
}

function cachePut(key: string, v: Uint8Array) {
  specCache.set(key, v);
  while (specCache.size > SPEC_CACHE_ENTRIES) specCache.delete(specCache.keys().next().value!);
}

/** Analysis frames a request covers beyond which it is fetched in parts. */
const FRAMES_PER_PART = 8000;

export function Spectrogram(p: LaneProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const image = useRef<{ canvas: HTMLCanvasElement; lit: number; topDb: number } | null>(null);
  const [levels, setLevels] = useState<SpecData | null>(null);
  const [drag, setDrag] = useState<[number, number, number, number] | null>(null);
  const [hover, setHover] = useState<string>("");
  const [pending, begin] = usePending();
  const height = 300;
  const fMin = 40;
  useWheel(ref, p);

  useEffect(() => {
    let live = true;
    if (p.width <= 0) return;
    const req = {
      t0: p.view.t0,
      t1: p.view.t1,
      columns: p.width,
      rows: height,
      f_min: p.view.scale === "log" ? fMin : 0,
      f_max: p.view.fMax,
      scale: p.view.scale,
      db_min: -200,
      db_max: 0,
      fft_at_48k: 2048,
    };
    const which = p.which;
    const meta = { which, t0: req.t0, t1: req.t1, columns: req.columns, rows: req.rows, fMax: req.f_max, scale: req.scale };
    const key = `${p.projectId}|${which}|${p.renderKey}|${JSON.stringify(req)}`;
    const hit = cacheGet(key);
    if (hit) {
      setLevels({ ...meta, values: hit, complete: true });
      p.onComplete?.();
      return;
    }
    // Long views are fetched in parts so the lane fills in from the left
    // instead of staying empty until the whole view is computed.
    const frames = ((req.t1 - req.t0) * p.sampleRate) / ((req.fft_at_48k * p.sampleRate) / 48000 / 4);
    const parts = Math.max(1, Math.min(req.columns, Math.ceil(frames / FRAMES_PER_PART)));
    const per = Math.ceil(req.columns / parts);
    const end = begin();
    (async () => {
      const values = new Uint8Array(req.rows * req.columns);
      for (let c0 = 0; c0 < req.columns; c0 += per) {
        const c1 = Math.min(req.columns, c0 + per);
        const part = await core.spectrogramPart(p.projectId, which, req, c0, c1);
        if (!live) return;
        const w = c1 - c0;
        for (let r = 0; r < req.rows; r++) values.set(part.subarray(r * w, (r + 1) * w), r * req.columns + c0);
        const complete = c1 === req.columns;
        if (complete) cachePut(key, values);
        setLevels({ ...meta, values, complete });
        if (complete) p.onComplete?.();
      }
    })().finally(end);
    return () => {
      live = false;
    };
  }, [p.projectId, p.which, p.view.t0, p.view.t1, p.view.fMax, p.view.scale, p.width, p.renderKey, p.sampleRate]);

  // Colour the levels once per arrival or range change. They arrive over −200…0 dB;
  // the display spans the top `dbRange` dB below the loudest cell.
  useEffect(() => {
    if (!levels) return;
    const { values, columns, rows } = levels;
    let top = 0;
    for (let i = 0; i < values.length; i++) if (values[i] > top) top = values[i];
    const topDb = (top / 255) * 200 - 200;
    const lo = topDb - p.view.dbRange;
    const canvas = image.current?.canvas ?? document.createElement("canvas");
    canvas.width = columns;
    canvas.height = rows;
    const g = canvas.getContext("2d")!;
    const img = g.createImageData(columns, rows);
    let lit = 0;
    for (let i = 0; i < values.length; i++) {
      const db = (values[i] / 255) * 200 - 200;
      const v = Math.max(0, Math.min(255, Math.round(((db - lo) / p.view.dbRange) * 255)));
      if (v > 16) lit++;
      img.data.set(INFERNO.subarray(v * 4, v * 4 + 4), i * 4);
    }
    g.putImageData(img, 0, 0);
    image.current = { canvas, lit, topDb };
  }, [levels, p.view.dbRange]);

  useEffect(() => {
    const c = ref.current;
    const im = image.current;
    if (!c || !levels || !im) return;
    const g = c.getContext("2d")!;
    g.fillStyle = "#000000";
    g.fillRect(0, 0, c.width, c.height);
    // Until new levels arrive after a zoom or scroll, the last image is drawn where it now falls
    // (and stretched vertically when only the top frequency changed on a linear scale).
    const dx = xAt(levels.t0, p.width, p.view);
    const dw = xAt(levels.t1, p.width, p.view) - dx;
    let dy = 0;
    let dh = height;
    if (levels.scale === "linear" && p.view.scale === "linear" && levels.fMax !== p.view.fMax) {
      dy = yAt(levels.fMax, height, p.view, fMin);
      dh = height - dy;
    }
    g.drawImage(im.canvas, dx, dy, dw, dh);
    const sel = drag
      ? {
          t0: timeAt(Math.min(drag[0], drag[2]), p.width, p.view),
          t1: timeAt(Math.max(drag[0], drag[2]), p.width, p.view),
          f_hi: freqAt(Math.min(drag[1], drag[3]), height, p.view, fMin),
          f_lo: freqAt(Math.max(drag[1], drag[3]), height, p.view, fMin),
        }
      : p.view.tfSelection;
    if (sel) {
      const a = xAt(sel.t0, p.width, p.view);
      const b = xAt(sel.t1, p.width, p.view);
      const y0 = yAt(sel.f_hi, height, p.view, fMin);
      const y1 = yAt(sel.f_lo, height, p.view, fMin);
      g.strokeStyle = "#7dffb3";
      g.lineWidth = 1.5;
      g.strokeRect(a, y0, b - a, y1 - y0);
      g.fillStyle = "rgba(125, 255, 179, 0.08)";
      g.fillRect(a, y0, b - a, y1 - y0);
    }
    const px = xAt(p.playhead, p.width, p.view);
    g.strokeStyle = "#ffffff";
    g.lineWidth = 1;
    g.beginPath();
    g.moveTo(px + 0.5, 0);
    g.lineTo(px + 0.5, height);
    g.stroke();
    debug("spectrogram", {
      columns: levels.columns,
      rows: levels.rows,
      litCells: im.lit,
      topDb: im.topDb,
      which: levels.which,
      t0: levels.t0,
      t1: levels.t1,
      complete: levels.complete,
    });
  }, [levels, p.view, p.playhead, p.width, drag]);

  const handlers = useDrag(
    (x) => p.onSeek(timeAt(x, p.width, p.view)),
    (x0, y0, x1, y1, done) => {
      if (done) {
        setDrag(null);
        p.onSelectTf({
          t0: timeAt(Math.min(x0, x1), p.width, p.view),
          t1: timeAt(Math.max(x0, x1), p.width, p.view),
          f_lo: Math.round(freqAt(Math.max(y0, y1), height, p.view, fMin)),
          f_hi: Math.round(freqAt(Math.min(y0, y1), height, p.view, fMin)),
        });
      } else setDrag([x0, y0, x1, y1]);
    },
  );

  return (
    <div className="spec-wrap">
      <canvas
        ref={ref}
        width={p.width}
        height={height}
        className="lane"
        data-testid="spectrogram"
        {...handlers}
        onMouseLeave={() => {
          handlers.onMouseLeave();
          setHover("");
        }}
        onMouseMove={(e) => {
          handlers.onMouseMove(e);
          const r = e.currentTarget.getBoundingClientRect();
          const x = e.clientX - r.left;
          const y = e.clientY - r.top;
          const t = timeAt(x, p.width, p.view);
          const f = freqAt(y, height, p.view, fMin);
          let db = "";
          // The level readout is only given when the drawn levels match the view exactly.
          if (
            levels &&
            levels.t0 === p.view.t0 &&
            levels.t1 === p.view.t1 &&
            levels.fMax === p.view.fMax &&
            levels.scale === p.view.scale
          ) {
            const v = levels.values[Math.floor(y) * levels.columns + Math.floor(x)];
            if (v !== undefined) db = ` · ${((v / 255) * 200 - 200).toFixed(1)} dB`;
          }
          setHover(`${t.toFixed(3)} s · ${f.toFixed(0)} Hz${db}`);
        }}
      />
      {hover && (
        <div className="hover" data-testid="hover">
          {hover}
        </div>
      )}
      <Busy on={pending} id="spectrogram-busy" />
    </div>
  );
}

export function TimeAxis({ view, width }: { view: ViewState; width: number }) {
  const span = view.t1 - view.t0;
  const steps = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 30, 60, 120, 300];
  const step = steps.find((s) => span / s <= width / 90) ?? 600;
  const ticks: number[] = [];
  for (let t = Math.ceil(view.t0 / step) * step; t <= view.t1; t += step) ticks.push(t);
  return (
    <div className="axis" style={{ width }}>
      {ticks.map((t) => {
        const x = ((t - view.t0) / span) * width;
        // Keep the first and last labels inside the lane.
        const shift = x < 24 ? "0" : x > width - 24 ? "-100%" : "-50%";
        return (
          <span key={t} style={{ left: x, transform: `translateX(${shift})` }}>
            {step < 1 ? t.toFixed(2) : t.toFixed(0)} s
          </span>
        );
      })}
    </div>
  );
}
