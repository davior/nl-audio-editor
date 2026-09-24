// The local library, in the browser's private file storage (OPFS).
//
//   nlae/projects/<id>/…            each project's files (manifest, log, lineage, caches)
//   nlae/blobs/<sha256>/<filename>  each source recording, stored once, under its original name
//
// Clones share their parent's source blob. The core decides what is written;
// this layer only persists it.
import type { Manifest } from "../core/types";

async function root(): Promise<FileSystemDirectoryHandle> {
  const r = await navigator.storage.getDirectory();
  return r.getDirectoryHandle("nlae", { create: true });
}

async function dir(base: FileSystemDirectoryHandle, parts: string[], create: boolean): Promise<FileSystemDirectoryHandle> {
  let d = base;
  for (const p of parts) d = await d.getDirectoryHandle(p, { create });
  return d;
}

async function writeFile(base: FileSystemDirectoryHandle, path: string, bytes: Uint8Array): Promise<void> {
  const parts = path.split("/");
  const name = parts.pop()!;
  const d = await dir(base, parts, true);
  const fh = await d.getFileHandle(name, { create: true });
  const w = await fh.createWritable();
  await w.write(bytes as unknown as FileSystemWriteChunkType);
  await w.close();
}

async function exists(base: FileSystemDirectoryHandle, path: string): Promise<boolean> {
  try {
    const parts = path.split("/");
    const name = parts.pop()!;
    const d = await dir(base, parts, false);
    await d.getFileHandle(name);
    return true;
  } catch {
    return false;
  }
}

async function readFile(base: FileSystemDirectoryHandle, path: string): Promise<Uint8Array> {
  const parts = path.split("/");
  const name = parts.pop()!;
  const d = await dir(base, parts, false);
  const f = await (await d.getFileHandle(name)).getFile();
  return new Uint8Array(await f.arrayBuffer());
}

async function walk(d: FileSystemDirectoryHandle, prefix: string, out: Record<string, Uint8Array>): Promise<void> {
  // @ts-expect-error: entries() is available on directory handles in Chromium.
  for await (const [name, handle] of d.entries()) {
    const path = prefix ? `${prefix}/${name}` : name;
    if (handle.kind === "directory") {
      await walk(handle as FileSystemDirectoryHandle, path, out);
    } else {
      const f = await (handle as FileSystemFileHandle).getFile();
      out[path] = new Uint8Array(await f.arrayBuffer());
    }
  }
}

function hex(sha: string): string {
  return sha.replace(/^sha256:/, "");
}

/** Persist files the core wrote. Source recordings go to the shared blob store. */
export async function persist(projectId: string, files: Record<string, Uint8Array>, manifest: Manifest): Promise<void> {
  const r = await root();
  const pdir = await dir(r, ["projects", projectId], true);
  for (const [path, bytes] of Object.entries(files)) {
    if (path.startsWith("renders/")) continue; // derived; recomputed on demand
    if (path.startsWith("source/")) {
      const blobPath = `blobs/${hex(manifest.source.sha256)}/${manifest.source.filename}`;
      if (!(await exists(r, blobPath))) await writeFile(r, blobPath, bytes);
      continue;
    }
    await writeFile(pdir, path, bytes);
  }
}

/** Every file of a project, with its source attached from the blob store. */
export async function load(projectId: string): Promise<Record<string, Uint8Array>> {
  const r = await root();
  const pdir = await dir(r, ["projects", projectId], false);
  const files: Record<string, Uint8Array> = {};
  await walk(pdir, "", files);
  const manifest = JSON.parse(new TextDecoder().decode(files["manifest.json"])) as Manifest;
  files[manifest.source.path] = await readFile(r, `blobs/${hex(manifest.source.sha256)}/${manifest.source.filename}`);
  return files;
}

export interface LibraryEntry {
  id: string;
  name: string;
  created: string;
  sourceSha: string;
  filename: string;
  duration: number;
  steps: number;
  parent: string | null;
  forkedAt: string | null;
}

export async function list(): Promise<LibraryEntry[]> {
  const r = await root();
  const out: LibraryEntry[] = [];
  let projects: FileSystemDirectoryHandle;
  try {
    projects = await r.getDirectoryHandle("projects");
  } catch {
    return out;
  }
  // @ts-expect-error: entries() is available on directory handles in Chromium.
  for await (const [id, handle] of projects.entries()) {
    if (handle.kind !== "directory") continue;
    try {
      const m = JSON.parse(new TextDecoder().decode(await readFile(handle as FileSystemDirectoryHandle, "manifest.json"))) as Manifest;
      out.push({
        id,
        name: m.project.name,
        created: m.project.created,
        sourceSha: m.source.sha256,
        filename: m.source.filename,
        duration: m.source_info.duration_s,
        steps: m.stack.steps.length,
        parent: m.lineage?.parent_project ?? null,
        forkedAt: m.lineage?.forked_at_step ?? null,
      });
    } catch {
      // Not a project (or unreadable); skip it.
    }
  }
  return out.sort((a, b) => a.created.localeCompare(b.created));
}
