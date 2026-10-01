import { describe, expect, it } from "vitest";
import type { Label } from "../core/types";
import { brief, inTimeOrder, isSelected, placeOf, placeWords, showLabel } from "./labels";
import { defaultView } from "./viewState";

const D = 12;
const SR = 48000;

const label = (over: Partial<Label> & Pick<Label, "t0" | "t1">): Label => ({ id: "lb", text: "x", seq: 1, ts: "", ...over });

describe("labels", () => {
  it("words a place, with its band for an area", () => {
    expect(placeWords({ t0: 1.25, t1: 1.75 })).toBe("1.250–1.750 s");
    expect(placeWords({ t0: 1, t1: 2, f_lo: 300, f_hi: 900 })).toBe("1.000–2.000 s × 300–900 Hz");
    expect(placeWords({ t0: 1, t1: 2, f_lo: null, f_hi: null })).toBe("1.000–2.000 s");
  });

  it("cuts long text short for a line of its own, whole characters only", () => {
    expect(brief("cough")).toBe("cough");
    expect(brief("x".repeat(40))).toBe("x".repeat(40));
    expect(brief("x".repeat(41))).toBe(`${"x".repeat(39)}…`);
    expect(brief("😀".repeat(50), 5)).toBe(`${"😀".repeat(4)}…`);
  });

  it("lists them in time order, ties in the order they were added", () => {
    const a = label({ id: "a", t0: 5, t1: 6, seq: 3 });
    const b = label({ id: "b", t0: 1, t1: 4, seq: 4 });
    const c = label({ id: "c", t0: 1, t1: 2, seq: 5 });
    const d = label({ id: "d", t0: 1, t1: 2, seq: 6 });
    expect(inTimeOrder([a, b, d, c]).map((l) => l.id)).toEqual(["c", "d", "b", "a"]);
    const input = [a, b];
    inTimeOrder(input);
    expect(input).toEqual([a, b]);
  });

  it("takes the place from the selection of its own kind, kept inside the recording", () => {
    const v = { ...defaultView(D, SR), selection: { t0: 1.5, t1: 2.5 }, tfSelection: { t0: 3, t1: 4, f_lo: 300, f_hi: 900 } };
    expect(placeOf("range", v, D, SR)).toEqual({ t0: 1.5, t1: 2.5 });
    expect(placeOf("area", v, D, SR)).toEqual({ t0: 3, t1: 4, f_lo: 300, f_hi: 900 });
    expect(placeOf("range", { ...v, selection: null }, D, SR)).toBeNull();
    expect(placeOf("area", { ...v, tfSelection: null }, D, SR)).toBeNull();
    // Rounding at the edges is brought back inside; nothing is left of a stretch outside.
    expect(placeOf("range", { ...v, selection: { t0: -1e-9, t1: D + 1e-9 } }, D, SR)).toEqual({ t0: 0, t1: D });
    expect(placeOf("range", { ...v, selection: { t0: D + 1, t1: D + 2 } }, D, SR)).toBeNull();
    expect(placeOf("area", { ...v, tfSelection: { t0: 3, t1: 4, f_lo: 100, f_hi: SR } }, D, SR)).toEqual({
      t0: 3,
      t1: 4,
      f_lo: 100,
      f_hi: SR / 2,
    });
  });

  it("selects a range again, and only that, bringing it into sight", () => {
    const v = {
      ...defaultView(D, SR),
      t0: 8,
      t1: 10,
      selection: { t0: 8.5, t1: 9 },
      tfSelection: { t0: 8.5, t1: 9, f_lo: 100, f_hi: 200 },
    };
    const l = label({ t0: 1.25, t1: 1.75 });
    const s = showLabel(v, l, D, SR);
    expect(s.selection).toEqual({ t0: 1.25, t1: 1.75 });
    expect(s.tfSelection).toBeNull();
    expect(s.t0).toBeLessThanOrEqual(1.25);
    expect(s.t1).toBeGreaterThanOrEqual(1.75);
    expect(s.t1 - s.t0).toBeCloseTo(2);
    expect(isSelected(l, s)).toBe(true);
    expect(isSelected(l, v)).toBe(false);
    // Already on screen: the window is not moved.
    const on = showLabel({ ...v, t0: 0, t1: 4 }, l, D, SR);
    expect([on.t0, on.t1]).toEqual([0, 4]);
  });

  it("selects an area again, and only that, raising the top frequency when it reaches above it", () => {
    const v = { ...defaultView(D, SR), selection: { t0: 1, t1: 2 } };
    const l = label({ t0: 6, t1: 7, f_lo: 3000, f_hi: 9500 });
    const s = showLabel(v, l, D, SR);
    expect(s.tfSelection).toEqual({ t0: 6, t1: 7, f_lo: 3000, f_hi: 9500 });
    expect(s.selection).toBeNull();
    expect(s.fMax).toBe(12000);
    expect(isSelected(l, s)).toBe(true);
    // Left alone when it fits, and never lowered.
    expect(showLabel(v, label({ t0: 1, t1: 2, f_lo: 100, f_hi: 900 }), D, SR).fMax).toBe(8000);
    expect(showLabel({ ...v, fMax: 24000 }, label({ t0: 1, t1: 2, f_lo: 100, f_hi: 900 }), D, SR).fMax).toBe(24000);
    // Up to the Nyquist frequency at most.
    expect(showLabel(v, label({ t0: 1, t1: 2, f_lo: 100, f_hi: 23000 }), D, SR).fMax).toBe(24000);
  });

  it("knows a label's place is selected only when the selection of its kind is exactly it", () => {
    const range = label({ t0: 1, t1: 2 });
    const area = label({ t0: 1, t1: 2, f_lo: 300, f_hi: 900 });
    const v = defaultView(D, SR);
    expect(isSelected(range, v)).toBe(false);
    expect(isSelected(range, { ...v, selection: { t0: 1, t1: 2 } })).toBe(true);
    expect(isSelected(range, { ...v, selection: { t0: 1, t1: 2.001 } })).toBe(false);
    // A time range does not stand for the area, nor the other way about.
    expect(isSelected(area, { ...v, selection: { t0: 1, t1: 2 } })).toBe(false);
    expect(isSelected(area, { ...v, tfSelection: { t0: 1, t1: 2, f_lo: 300, f_hi: 900 } })).toBe(true);
    expect(isSelected(area, { ...v, tfSelection: { t0: 1, t1: 2, f_lo: 300, f_hi: 901 } })).toBe(false);
    expect(isSelected(range, { ...v, tfSelection: { t0: 1, t1: 2, f_lo: 300, f_hi: 900 } })).toBe(false);
  });
});
