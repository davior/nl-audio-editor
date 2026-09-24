import * as Comlink from "comlink";
import type { AnalysisApi, AnalysisResult } from "./analysis-worker";
import type { Pcm } from "./types";
import type { CoreApi } from "./worker";

const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
export const core = Comlink.wrap<CoreApi>(worker);

/**
 * Analyse decoded audio in a worker of its own, which is released afterwards
 * (a WebAssembly instance never gives memory back while it lives).
 */
export async function analyseInBackground(pcm: Pcm): Promise<AnalysisResult> {
  const w = new Worker(new URL("./analysis-worker.ts", import.meta.url), { type: "module" });
  const api = Comlink.wrap<AnalysisApi>(w);
  try {
    return await api.analyse(
      Comlink.transfer(
        pcm,
        pcm.channels.map((c) => c.buffer as ArrayBuffer),
      ),
    );
  } finally {
    api[Comlink.releaseProxy]();
    w.terminate();
  }
}

/** Hand byte buffers to the worker without copying them. */
export function transferFiles(files: Record<string, Uint8Array>): Record<string, Uint8Array> {
  const buffers = new Set(Object.values(files).map((b) => b.buffer as ArrayBuffer));
  return Comlink.transfer(files, [...buffers]);
}
