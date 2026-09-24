import { describe, expect, it } from "vitest";
import { defaultView, follow, MIN_SPAN, restoreView, scroll, showRange, zoom } from "./viewState";

const D = 12;
const SR = 48000;

describe("view state", () => {
  it("starts on the whole recording up to 8 kHz", () => {
    const v = defaultView(D, SR);
    expect([v.t0, v.t1, v.fMax, v.monitor]).toEqual([0, D, 8000, "source"]);
    expect(defaultView(D, 8000).fMax).toBe(4000);
  });

  it("restores a saved view exactly and rejects values that do not fit the recording", () => {
    const saved = {
      t0: 1.2345678901234,
      t1: 3.5,
      scale: "log",
      fMax: 4000,
      dbRange: 60,
      vZoom: 32,
      monitor: "residual",
      selection: { t0: 2, t1: 2.5 },
      tfSelection: { t0: 1.5, t1: 3, f_lo: 700, f_hi: 800 },
    };
    expect(restoreView(saved, D, SR)).toEqual(saved);
    const bad = restoreView({ t0: -1, t1: 99, scale: "cubic", fMax: 96000, monitor: "x", selection: { t0: 3, t1: 1 } }, D, SR);
    expect(bad).toEqual(defaultView(D, SR));
    expect(restoreView(undefined, D, SR)).toEqual(defaultView(D, SR));
  });

  it("zooms around the pointer and stays inside the recording", () => {
    const v = defaultView(D, SR);
    const z = zoom(v, 0.5, 3, D);
    expect(z.t1 - z.t0).toBeCloseTo(6);
    // The time under the pointer keeps its place on screen.
    expect((3 - z.t0) / (z.t1 - z.t0)).toBeCloseTo(3 / 12);
    const out = zoom(z, 100, 3, D);
    expect([out.t0, out.t1]).toEqual([0, D]);
    let tiny = v;
    for (let i = 0; i < 40; i++) tiny = zoom(tiny, 0.5, 11.99, D);
    expect(tiny.t1 - tiny.t0).toBeCloseTo(MIN_SPAN);
    expect(tiny.t1).toBeLessThanOrEqual(D);
  });

  it("scrolls without leaving the recording", () => {
    const v = { ...defaultView(D, SR), t0: 2, t1: 4 };
    expect(scroll(v, -5, D)).toMatchObject({ t0: 0, t1: 2 });
    expect(scroll(v, 50, D)).toMatchObject({ t0: 10, t1: 12 });
  });

  it("shows a range with a margin and pages forward during playback", () => {
    const v = defaultView(D, SR);
    const r = showRange(v, 4, 6, D);
    expect(r.t0).toBeCloseTo(3.9);
    expect(r.t1).toBeCloseTo(6.1);
    const w = { ...v, t0: 0, t1: 2 };
    expect(follow(w, 1, D)).toBe(w);
    expect(follow(w, 2.01, D)).toMatchObject({ t0: 2.01 });
  });
});
