// Persist what the core wrote since the last save. Saves run one at a time so
// two quick changes can never interleave their writes.
import { core } from "../core/client";
import type { Manifest } from "../core/types";
import { persist } from "./opfs";

let queue: Promise<void> = Promise.resolve();

export function sync(projectId: string, manifest: Manifest): Promise<void> {
  const run = queue.then(async () => {
    const files = await core.takeChangedFiles(projectId);
    if (Object.keys(files).length > 0) await persist(projectId, files, manifest);
  });
  queue = run.catch(() => undefined);
  return run;
}
