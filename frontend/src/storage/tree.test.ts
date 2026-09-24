import { describe, expect, it } from "vitest";
import type { LibraryEntry } from "./opfs";
import { bytesEqual, families, isPrefix } from "./tree";

const entry = (id: string, sha: string, created: string, parent: string | null = null): LibraryEntry => ({
  id,
  name: id,
  created,
  sourceSha: sha,
  filename: `${sha}.wav`,
  duration: 1,
  steps: 0,
  parent,
  forkedAt: null,
});

describe("library families", () => {
  it("groups by source and nests clones under their parent", () => {
    const g = families([
      entry("b1", "b", "2026-01-03"),
      entry("a1", "a", "2026-01-01"),
      entry("a2", "a", "2026-01-02", "a1"),
      entry("a3", "a", "2026-01-04", "a2"),
      entry("a4", "a", "2026-01-05", "a1"),
      entry("orphan", "a", "2026-01-06", "gone"),
    ]);
    expect(g.map((x) => x.sourceSha)).toEqual(["a", "b"]);
    const a = g[0];
    expect(a.roots.map((n) => n.entry.id)).toEqual(["a1", "orphan"]);
    expect(a.roots[0].children.map((n) => n.entry.id)).toEqual(["a2", "a4"]);
    expect(a.roots[0].children[0].children.map((n) => n.entry.id)).toEqual(["a3"]);
  });

  it("compares logs byte for byte", () => {
    const a = new TextEncoder().encode("line 1\nline 2\n");
    const b = new TextEncoder().encode("line 1\nline 2\nline 3\n");
    expect(bytesEqual(a, a.slice())).toBe(true);
    expect(isPrefix(a, b)).toBe(true);
    expect(isPrefix(b, a)).toBe(false);
    const c = new TextEncoder().encode("line 1\nline X\nline 3\n");
    expect(isPrefix(a, c)).toBe(false);
  });
});
