import type { Manifest } from "../../../shared/types/Manifest";
import type { OpenReport } from "../../../shared/types/OpenReport";
import type { StackState } from "../../../shared/types/StackState";
import type { SpectrogramRequest } from "../../../shared/types/SpectrogramRequest";

export type { Manifest, OpenReport, StackState };
export type { Step } from "../../../shared/types/Step";
export type { Scope } from "../../../shared/types/Scope";

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

export type Which = "source" | "stack" | "residual";

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
