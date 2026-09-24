import * as Comlink from "comlink";
import type { AnalysisApi, AnalysisResult } from "./analysis-worker";
import type { Pcm } from "./types";
import type { CoreApi } from "./worker";

const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
export const core = Comlink.wrap<CoreApi>(worker);

// A worker whose code cannot load never answers, so every call to it would
// wait for ever; say why instead. Running from source, the usual cause is a
// core that has not been built yet.
let coreFailure: Error | null = null;
const failureListeners = new Set<(e: Error) => void>();
worker.addEventListener("error", (e) => {
  coreFailure ??= new Error(
    e instanceof ErrorEvent
      ? `The audio core stopped: ${e.message}`
      : "The audio core did not load. If you are running from source, build it with `pnpm run wasm` (in frontend/), then reload the page.",
  );
  failureListeners.forEach((f) => f(coreFailure!));
});

/** Calls `f` if the core worker fails (at once if it already has); returns an unsubscribe function. */
export function onCoreFailure(f: (e: Error) => void): () => void {
  if (coreFailure) f(coreFailure);
  failureListeners.add(f);
  return () => {
    failureListeners.delete(f);
  };
}

/**
 * Analyse decoded audio in a worker of its own, which is released afterwards
 * (a WebAssembly instance never gives memory back while it lives).
 */
export async function analyseInBackground(pcm: Pcm): Promise<AnalysisResult> {
  const w = new Worker(new URL("./analysis-worker.ts", import.meta.url), { type: "module" });
  const api = Comlink.wrap<AnalysisApi>(w);
  const failed = new Promise<never>((_, reject) =>
    w.addEventListener(
      "error",
      (e) => reject(new Error(e instanceof ErrorEvent ? `The analysis stopped: ${e.message}` : "The analysis worker did not load.")),
      { once: true },
    ),
  );
  failed.catch(() => {}); // settled after the race is decided: not an unhandled rejection
  try {
    return await Promise.race([
      api.analyse(
        Comlink.transfer(
          pcm,
          pcm.channels.map((c) => c.buffer as ArrayBuffer),
        ),
      ),
      failed,
    ]);
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
