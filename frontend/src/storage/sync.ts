// Persist what the core wrote since the last save. Saves run one at a time so
// two quick changes can never interleave their writes.
import { core } from "../core/client";
import type { Manifest } from "../core/types";
import { persist } from "./opfs";

let queue: Promise<void> = Promise.resolve();

export interface SyncOptions {
  /** Leave the recording out (it is stored some other way, e.g. `before`). */
  skipSource?: boolean;
  /** Runs first, inside the same turn of the queue. */
  before?: () => Promise<void>;
}

export function sync(projectId: string, manifest: Manifest, opts: SyncOptions = {}): Promise<void> {
  const run = queue.then(async () => {
    await opts.before?.();
    const files = await core.takeChangedFiles(projectId, opts.skipSource ?? false);
    if (Object.keys(files).length > 0) await persist(projectId, files, manifest);
  });
  queue = run.catch(() => undefined);
  return run;
}
