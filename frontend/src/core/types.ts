import type { Manifest } from "../../../shared/types/Manifest";
import type { OpenReport } from "../../../shared/types/OpenReport";
import type { StackState } from "../../../shared/types/StackState";
import type { SpectrogramRequest } from "../../../shared/types/SpectrogramRequest";

export type { Manifest, OpenReport, StackState };
export type { Step } from "../../../shared/types/Step";
export type { Scope } from "../../../shared/types/Scope";
export type { PreviewRecord } from "../../../shared/types/PreviewRecord";
export type { Route } from "../../../shared/types/Route";
export type { RoutedStep } from "../../../shared/types/RoutedStep";
export type { Selection } from "../../../shared/types/Selection";
export type { Turn } from "../../../shared/types/Turn";
export type { Proposal } from "../../../shared/types/Proposal";
export type { Exchange } from "../../../shared/types/Exchange";
export type { Dictation } from "../../../shared/types/Dictation";
export type { Segment } from "../../../shared/types/Segment";

/** A just-created or cloned project has no open report yet; only a list of problems. */
export type Report = OpenReport | { problems: string[]; created?: boolean; cloned?: boolean };

export interface ProjectSummary {
  id: string;
  manifest: Manifest;
  report: Report;
  state: StackState;
  /** Set when the project did not verify: nothing can be written to it. */
  readOnly: string | null;
  /** The source has no analysis of the current version logged yet. */
  needsAnalysis: boolean;
}

export type Which =
  | "source"
  | "stack"
  | "output"
  | "residual"
  | "preview:original"
  | "preview:before"
  | "preview:output"
  | "preview:residual";

/** A piece of the output: original audio, or inserted silence (seconds). */
export type EditPiece =
  | { kind: "kept"; src0: number; src1: number; out0: number }
  | { kind: "silence"; at: number; len: number; out0: number };

/** Where a time edit shows (seconds of the original, and of the output). */
export type EditMark =
  | { kind: "removed"; from_s: number; to_s: number; duration_s: number; at_output_s: number }
  | { kind: "inserted"; at_s: number; duration_s: number; at_output_s: number };

/** How the stack's time edits lay the original out in the output. */
export interface EditMap {
  edited: boolean;
  duration_s: number;
  original_duration_s: number;
  pieces: EditPiece[];
  marks: EditMark[];
}

export interface Pcm {
  sampleRate: number;
  channels: Float32Array[];
}

export type SpectrogramReq = SpectrogramRequest;

export interface ViewState {
  t0: number;
  t1: number;
  scale: "linear" | "log";
  fMax: number;
  dbRange: number;
  vZoom: number;
  monitor: Which;
  selection: { t0: number; t1: number } | null;
  tfSelection: { t0: number; t1: number; f_lo: number; f_hi: number } | null;
}

/** A preview the core has open: its record and window (seconds). */
export interface PreviewResult {
  record: import("../../../shared/types/PreviewRecord").PreviewRecord;
  window: [number, number];
  summary: ProjectSummary;
}
