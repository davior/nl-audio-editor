/// <reference lib="webworker" />
// The core runs here, off the main thread: decoding, analysis, rendering and
// every change to a project. The page only draws and plays what comes back.
import * as Comlink from "comlink";
import init, { WasmProject, parity, descriptors } from "../core-wasm/nlae.js";
import wasmUrl from "../core-wasm/nlae_bg.wasm?url";
import type { ProjectSummary, Pcm, Which, SpectrogramReq } from "./types";

const ready = init({ module_or_path: wasmUrl });
const projects = new Map<string, WasmProject>();

function get(id: string): WasmProject {
  const p = projects.get(id);
  if (!p) throw new Error(`project ${id} is not open`);
  return p;
}

function summary(p: WasmProject): ProjectSummary {
  return { id: p.id(), manifest: p.manifest(), report: p.report(), state: p.state(), readOnly: p.read_only() ?? null };
}

function keep(p: WasmProject): ProjectSummary {
  const old = projects.get(p.id());
  if (old && old !== p) old.free();
  projects.set(p.id(), p);
  return summary(p);
}

const api = {
  async importFile(bytes: Uint8Array, filename: string, lastModified?: string): Promise<ProjectSummary> {
    await ready;
    return keep(WasmProject.create(bytes, filename, undefined, lastModified, undefined));
  },
  async importRecording(bytes: Uint8Array, filename: string, capture: unknown): Promise<ProjectSummary> {
    await ready;
    return keep(WasmProject.create(bytes, filename, undefined, undefined, JSON.stringify(capture)));
  },
  async openProject(files: Record<string, Uint8Array>): Promise<ProjectSummary> {
    await ready;
    return keep(WasmProject.open(files));
  },
  async openBundle(bytes: Uint8Array): Promise<ProjectSummary> {
    await ready;
    return keep(WasmProject.open_bundle(bytes));
  },
  async summary(id: string): Promise<ProjectSummary> {
    return summary(get(id));
  },
  async events(id: string): Promise<Record<string, unknown>[]> {
    return get(id).events();
  },
  async takeChangedFiles(id: string): Promise<Record<string, Uint8Array>> {
    return get(id).take_changed_files();
  },
  async allFiles(id: string): Promise<Record<string, Uint8Array>> {
    return get(id).all_files();
  },
  async pcm(id: string, which: Which): Promise<Pcm> {
    const r = get(id).pcm(which) as Pcm;
    return Comlink.transfer(
      r,
      r.channels.map((c) => c.buffer as ArrayBuffer),
    );
  },
  async peaks(id: string, which: Which, t0: number, t1: number, columns: number): Promise<Float32Array> {
    const r = get(id).peaks(which, t0, t1, columns);
    return Comlink.transfer(r, [r.buffer as ArrayBuffer]);
  },
  async spectrogram(id: string, which: Which, req: SpectrogramReq): Promise<Uint8Array> {
    const r = get(id).spectrogram(which, req);
    return Comlink.transfer(r, [r.buffer as ArrayBuffer]);
  },
  async renderHash(id: string, which: Which): Promise<string> {
    return get(id).render_hash(which);
  },
  async features(id: string, which: Which): Promise<Record<string, unknown>> {
    return get(id).features(which);
  },
  async setView(id: string, view: unknown): Promise<void> {
    get(id).set_view(view);
  },
  async cloneProject(id: string, atStep?: string, name?: string): Promise<ProjectSummary> {
    return keep(get(id).clone_project(atStep, name));
  },
  async exportBundle(id: string): Promise<Uint8Array> {
    const r = get(id).export_bundle();
    return Comlink.transfer(r, [r.buffer as ArrayBuffer]);
  },
  async close(id: string): Promise<void> {
    projects.get(id)?.free();
    projects.delete(id);
  },
  async parity(): Promise<Record<string, string>> {
    await ready;
    return parity();
  },
  async descriptors(): Promise<unknown[]> {
    await ready;
    return descriptors();
  },
};

export type CoreApi = typeof api;
Comlink.expose(api);
