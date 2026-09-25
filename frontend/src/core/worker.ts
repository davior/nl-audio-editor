/// <reference lib="webworker" />
// The core runs here, off the main thread: decoding, analysis, rendering and
// every change to a project. The page only draws and plays what comes back.
import * as Comlink from "comlink";
import init, { WasmProject, parity, descriptors, route, next_round, listen_params } from "../core-wasm/nlae.js";
import wasmUrl from "../core-wasm/nlae_bg.wasm?url";
import type {
  Dictation,
  EditMap,
  Exchange,
  NextRound,
  Pcm,
  PreviewRecord,
  PreviewResult,
  ProjectSummary,
  Proposal,
  Rounds,
  Route,
  RoutedStep,
  Selection,
  SpectrogramReq,
  Turn,
  Which,
} from "./types";

const ready = init({ module_or_path: wasmUrl });

/** Hand byte buffers back without copying them. */
function transferFiles(files: Record<string, Uint8Array>): Record<string, Uint8Array> {
  const buffers = new Set(Object.values(files).map((b) => b.buffer as ArrayBuffer));
  return Comlink.transfer(files, [...buffers]);
}
const projects = new Map<string, WasmProject>();

function get(id: string): WasmProject {
  const p = projects.get(id);
  if (!p) throw new Error(`project ${id} is not open`);
  return p;
}

function summary(p: WasmProject): ProjectSummary {
  return {
    id: p.id(),
    manifest: p.manifest(),
    report: p.report(),
    state: p.state(),
    readOnly: p.read_only() ?? null,
    needsAnalysis: p.needs_analysis(),
  };
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
  async takeChangedFiles(id: string, skipSource = false): Promise<Record<string, Uint8Array>> {
    return transferFiles(get(id).take_changed_files(skipSource));
  },
  async allFiles(id: string): Promise<Record<string, Uint8Array>> {
    return transferFiles(get(id).all_files());
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
  /** Columns `[c0, c1)` of a spectrogram request (rows × (c1 − c0) levels). */
  async spectrogramPart(id: string, which: Which, req: SpectrogramReq, c0: number, c1: number): Promise<Uint8Array> {
    const r = get(id).spectrogram_part(which, req, c0, c1);
    return Comlink.transfer(r, [r.buffer as ArrayBuffer]);
  },
  /** Log an analysis made by the analysis worker, after the core checks it describes this source. */
  async recordAnalysis(id: string, features: Record<string, unknown>, renderHash: string): Promise<ProjectSummary> {
    const p = get(id);
    p.record_analysis(JSON.stringify(features), renderHash);
    return summary(p);
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
  // --- The console (M1) ---
  async route(words: string, selection: Selection | null): Promise<Route> {
    await ready;
    return route(words, selection);
  },
  /** What a model's answer calls for next, by the core's policy: use it, send a follow-up (JSON text), or give up. */
  async nextRound(request: string, response: string, rounds: Rounds): Promise<NextRound> {
    await ready;
    return next_round(request, response, rounds);
  },
  /** The query for streaming dictation, as `[name, value]` pairs; the key is not part of it. */
  async listenParams(model: string, language: string, sampleRate: number, optOut: boolean): Promise<[string, string][]> {
    await ready;
    return listen_params(model, language, sampleRate, optOut);
  },
  /** Log a spoken request before acting on it; returns the event's hash, for the preview to refer to. */
  async recordDictation(id: string, dictation: Dictation): Promise<string> {
    return get(id).record_dictation(dictation);
  },
  async previewRouted(id: string, steps: RoutedStep[], words: string, dictation?: string): Promise<PreviewResult> {
    const p = get(id);
    const r = p.preview_routed(steps, words, dictation) as { record: PreviewRecord; window: [number, number] };
    return { ...r, summary: summary(p) };
  },
  async previewRecipe(id: string, name: string, words: string, dictation?: string): Promise<PreviewResult> {
    const p = get(id);
    const r = p.preview_recipe(name, words, dictation) as { record: PreviewRecord; window: [number, number] };
    return { ...r, summary: summary(p) };
  },
  async assistantRequest(id: string, model: string, history: Turn[], words: string, selection: Selection | null): Promise<string> {
    return get(id).assistant_request(model, history, words, selection);
  },
  async recordExchange(id: string, exchange: Exchange): Promise<string> {
    return get(id).record_exchange(exchange);
  },
  async previewProposal(
    id: string,
    proposal: Proposal,
    words: string,
    model: string,
    provider: string,
    exchange: string,
    dictation?: string,
  ): Promise<PreviewResult> {
    const p = get(id);
    const r = p.preview_proposal(proposal, words, model, provider, exchange, dictation) as {
      record: PreviewRecord;
      window: [number, number];
    };
    return { ...r, summary: summary(p) };
  },
  async accept(
    id: string,
    previewId: string,
    overrides: Record<number, Record<string, unknown>>,
    disabled: number[],
    note?: string,
  ): Promise<ProjectSummary> {
    const p = get(id);
    p.accept(previewId, overrides, disabled, note);
    return summary(p);
  },
  async reject(id: string, previewId: string, reason?: string): Promise<ProjectSummary> {
    const p = get(id);
    p.reject(previewId, reason || undefined);
    return summary(p);
  },
  async removeTop(id: string): Promise<ProjectSummary> {
    const p = get(id);
    p.remove_top();
    return summary(p);
  },
  async editMap(id: string): Promise<EditMap> {
    return get(id).edit_map() as EditMap;
  },
  async rateStack(id: string, overall: number, note?: string): Promise<ProjectSummary> {
    const p = get(id);
    p.rate_stack(overall, note || undefined);
    return summary(p);
  },
  async exportWav(id: string, format: "f32" | "pcm24" | "pcm16"): Promise<Uint8Array> {
    const r = get(id).export_wav(format);
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
