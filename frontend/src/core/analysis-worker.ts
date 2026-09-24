/// <reference lib="webworker" />
// Analysis runs here, beside the core worker, so the lanes stay responsive
// while a long recording is analysed. It sees only the decoded samples it is
// given; the core worker checks the result describes its source before logging it.
import * as Comlink from "comlink";
import init, { analyse_pcm } from "../core-wasm/nlae.js";
import wasmUrl from "../core-wasm/nlae_bg.wasm?url";
import type { Pcm } from "./types";

const ready = init({ module_or_path: wasmUrl });

export interface AnalysisResult {
  features: Record<string, unknown>;
  renderHash: string;
}

const api = {
  async analyse(pcm: Pcm): Promise<AnalysisResult> {
    await ready;
    const r = analyse_pcm(pcm.sampleRate, pcm.channels) as { features: Record<string, unknown>; render_hash: string };
    return { features: r.features, renderHash: r.render_hash };
  },
};

export type AnalysisApi = typeof api;
Comlink.expose(api);
