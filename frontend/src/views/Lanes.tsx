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
  revision: number;
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

export function Waveform(p: LaneProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [peaks, setPeaks] = useState<Float32Array | null>(null);
  const [drag, setDrag] = useState<[number, number] | null>(null);
  const height = 140;
  useWheel(ref, p);

  useEffect(() => {
    let live = true;
    if (p.width <= 0) return;
    core.peaks(p.projectId, p.which, p.view.t0, p.view.t1, p.width).then((pk) => live && setPeaks(pk));
    return () => {
      live = false;
    };
  }, [p.projectId, p.which, p.view.t0, p.view.t1, p.width, p.revision]);

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
    for (let x = 0; x < p.width; x++) {
      const lo = peaks[2 * x] * p.view.vZoom;
      const hi = peaks[2 * x + 1] * p.view.vZoom;
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
    debug("waveform", { columns: p.width, columnsWithSignal: drawn });
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

  return <canvas ref={ref} width={p.width} height={height} className="lane" data-testid="waveform" {...handlers} />;
}

export function Spectrogram(p: LaneProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [levels, setLevels] = useState<Uint8Array | null>(null);
  const [drag, setDrag] = useState<[number, number, number, number] | null>(null);
  const [hover, setHover] = useState<string>("");
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
    core.spectrogram(p.projectId, p.which, req).then((l) => live && setLevels(l));
    return () => {
      live = false;
    };
  }, [p.projectId, p.which, p.view.t0, p.view.t1, p.view.fMax, p.view.scale, p.width, p.revision]);

  useEffect(() => {
    const c = ref.current;
    if (!c || !levels) return;
    const g = c.getContext("2d")!;
    // Levels arrive over −200…0 dB; display the top `dbRange` dB below the loudest cell.
    let top = 0;
    for (let i = 0; i < levels.length; i++) if (levels[i] > top) top = levels[i];
    const topDb = (top / 255) * 200 - 200;
    const lo = topDb - p.view.dbRange;
    const img = g.createImageData(p.width, height);
    let lit = 0;
    for (let i = 0; i < levels.length; i++) {
      const db = (levels[i] / 255) * 200 - 200;
      const v = Math.max(0, Math.min(255, Math.round(((db - lo) / p.view.dbRange) * 255)));
      if (v > 16) lit++;
      img.data.set(INFERNO.subarray(v * 4, v * 4 + 4), i * 4);
    }
    g.putImageData(img, 0, 0);
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
    debug("spectrogram", { columns: p.width, rows: height, litCells: lit, topDb });
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
        onMouseMove={(e) => {
          handlers.onMouseMove(e);
          const r = e.currentTarget.getBoundingClientRect();
          const x = e.clientX - r.left;
          const y = e.clientY - r.top;
          const t = timeAt(x, p.width, p.view);
          const f = freqAt(y, height, p.view, fMin);
          let db = "";
          if (levels) {
            const v = levels[Math.floor(y) * p.width + Math.floor(x)];
            if (v !== undefined) db = `${((v / 255) * 200 - 200).toFixed(1)} dB`;
          }
          setHover(`${t.toFixed(3)} s · ${f.toFixed(0)} Hz · ${db}`);
        }}
      />
      <div className="hover" data-testid="hover">
        {hover}
      </div>
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
