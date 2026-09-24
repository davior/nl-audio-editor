// The library as families: projects grouped by source recording, each clone
// under the project it was cloned from.
import type { LibraryEntry } from "./opfs";

export interface TreeNode {
  entry: LibraryEntry;
  children: TreeNode[];
}

export interface SourceGroup {
  sourceSha: string;
  filename: string;
  duration: number;
  roots: TreeNode[];
}

export function families(entries: LibraryEntry[]): SourceGroup[] {
  const bySource = new Map<string, LibraryEntry[]>();
  for (const e of entries) {
    const g = bySource.get(e.sourceSha) ?? [];
    g.push(e);
    bySource.set(e.sourceSha, g);
  }
  const groups: SourceGroup[] = [];
  for (const [sha, list] of bySource) {
    const sorted = [...list].sort((a, b) => a.created.localeCompare(b.created) || a.id.localeCompare(b.id));
    const nodes = new Map(sorted.map((e) => [e.id, { entry: e, children: [] as TreeNode[] }]));
    const roots: TreeNode[] = [];
    for (const e of sorted) {
      const node = nodes.get(e.id)!;
      const parent = e.parent ? nodes.get(e.parent) : undefined;
      // A clone whose parent is not in the library is shown at the top level.
      if (parent && parent !== node) parent.children.push(node);
      else roots.push(node);
    }
    groups.push({ sourceSha: sha, filename: sorted[0].filename, duration: sorted[0].duration, roots });
  }
  return groups.sort((a, b) => a.roots[0].entry.created.localeCompare(b.roots[0].entry.created));
}

export function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/** True when `a` is a prefix of `b` (b continues a's history). */
export function isPrefix(a: Uint8Array, b: Uint8Array): boolean {
  return a.length <= b.length && bytesEqual(a, b.subarray(0, a.length));
}
