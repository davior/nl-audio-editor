// Original time ↔ output time under the stack's time edits. The lanes always
// show the original's timeline; the output skips removed stretches and has
// silence inserted, so playback maps between the two.
import type { EditPiece } from "../core/types";

export class TimeMap {
  constructor(private readonly pieces: EditPiece[]) {}

  private static length(p: EditPiece): number {
    return p.kind === "kept" ? p.src1 - p.src0 : p.len;
  }

  /**
   * Output time of an original time. Inside a removed stretch: the join after
   * it. At a point where silence was inserted: the start of the silence.
   */
  toOutput(t: number): number {
    let joinOut = 0;
    for (const p of this.pieces) {
      if (p.kind === "silence") {
        if (Math.abs(t - p.at) < 1e-9) return p.out0;
        continue;
      }
      if (t < p.src0) return joinOut;
      if (t < p.src1) return p.out0 + (t - p.src0);
      joinOut = p.out0 + (p.src1 - p.src0);
    }
    return this.pieces.length ? this.end() : t;
  }

  /** Original time of an output time; during inserted silence, where it was inserted. */
  toOriginal(o: number): number {
    for (const p of this.pieces) {
      if (o < p.out0 + TimeMap.length(p)) return p.kind === "kept" ? p.src0 + Math.max(0, o - p.out0) : p.at;
    }
    const last = this.pieces[this.pieces.length - 1];
    if (!last) return o;
    return last.kind === "kept" ? last.src1 : last.at;
  }

  /** Output length. */
  end(): number {
    const last = this.pieces[this.pieces.length - 1];
    return last ? last.out0 + TimeMap.length(last) : 0;
  }
}
