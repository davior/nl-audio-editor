import { describe, expect, it } from "vitest";
import { TimeMap } from "./timemap";

// 10 s original: 2–3 s removed, 0.5 s of silence inserted at 6 s.
const map = new TimeMap([
  { kind: "kept", src0: 0, src1: 2, out0: 0 },
  { kind: "kept", src0: 3, src1: 6, out0: 2 },
  { kind: "silence", at: 6, len: 0.5, out0: 5 },
  { kind: "kept", src0: 6, src1: 10, out0: 5.5 },
]);

describe("TimeMap", () => {
  it("maps kept audio both ways", () => {
    expect(map.toOutput(1)).toBe(1);
    expect(map.toOutput(4)).toBe(3);
    expect(map.toOriginal(3)).toBe(4);
    expect(map.toOutput(7)).toBe(6.5);
    expect(map.toOriginal(6.5)).toBe(7);
  });
  it("sends a removed stretch to its join, and holds during inserted silence", () => {
    expect(map.toOutput(2.5)).toBe(2);
    expect(map.toOutput(6)).toBe(5);
    expect(map.toOriginal(5.25)).toBe(6);
    expect(map.end()).toBe(9.5);
    expect(map.toOriginal(9.5)).toBe(10);
  });
});
