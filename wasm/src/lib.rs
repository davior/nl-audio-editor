//! WebAssembly bindings over the core, used from a Web Worker.
//!
//! A `WasmProject` holds one project in memory. Files the project writes are
//! tracked so the page can persist exactly what changed to the browser's
//! private file storage (OPFS): the source once, the log as it grows, the
//! manifest when it changes.

use std::collections::{BTreeMap, BTreeSet};

use js_sys::{Float32Array, Object, Reflect, Uint8Array};
use nlae_core::analysis::spectrogram::{
    spectrogram as draw, spectrogram_columns, SpectrogramRequest,
};
use nlae_core::analysis::{features, peaks::peaks};
use nlae_core::assistant::{self, Exchange, Proposal, RoutedStep, Selection, Turn};
use nlae_core::audio::decode;
use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::project::store::{MemStore, Store, StoreError};
use nlae_core::project::{
    bundle, AcceptOptions, CreateOptions, Origin, Preview, PreviewOptions, Project,
};
use nlae_core::provenance::env::{format_rfc3339_ms, ulid};
use nlae_core::provenance::Actor;
use nlae_core::provenance::{AppInfo, Env};
use nlae_core::recipe::{Recipe, ReplayMode};
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

fn to_js<T: Serialize>(v: &T) -> Result<JsValue, JsValue> {
    let ser = serde_wasm_bindgen::Serializer::json_compatible();
    v.serialize(&ser).map_err(js_err)
}

fn app() -> AppInfo {
    AppInfo::new("web")
}

/// Browser clock and identifiers.
struct WebEnv;

impl Env for WebEnv {
    fn now(&mut self) -> String {
        format_rfc3339_ms(js_sys::Date::now() as u64)
    }

    fn new_id(&mut self, prefix: &str) -> String {
        let mut rnd = [0u8; 10];
        for b in rnd.iter_mut() {
            *b = (js_sys::Math::random() * 256.0) as u8;
        }
        format!(
            "{prefix}_{}",
            ulid(js_sys::Date::now() as u64, rnd).to_lowercase()
        )
    }
}

/// A memory store that remembers which files changed.
#[derive(Clone, Default)]
pub struct TrackingStore {
    inner: MemStore,
    dirty: BTreeSet<String>,
}

impl Store for TrackingStore {
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.inner.read(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }
    fn write_new(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        self.inner.write_new(path, data)?;
        self.dirty.insert(path.to_string());
        Ok(())
    }
    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        self.inner.append(path, data)?;
        self.dirty.insert(path.to_string());
        Ok(())
    }
    fn replace(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        self.inner.replace(path, data)?;
        self.dirty.insert(path.to_string());
        Ok(())
    }
    fn list(&self) -> Vec<String> {
        self.inner.list()
    }
}

fn files_object(store: &MemStore, paths: impl Iterator<Item = String>) -> Result<JsValue, JsValue> {
    let obj = Object::new();
    for p in paths {
        if let Ok(bytes) = store.read(&p) {
            Reflect::set(
                &obj,
                &JsValue::from_str(&p),
                &Uint8Array::from(bytes.as_slice()),
            )?;
        }
    }
    Ok(obj.into())
}

fn planar(audio: &nlae_core::audio::AudioBuffer) -> Result<JsValue, JsValue> {
    let obj = Object::new();
    Reflect::set(
        &obj,
        &"sampleRate".into(),
        &JsValue::from(audio.sample_rate),
    )?;
    let chans = js_sys::Array::new();
    for c in &audio.channels {
        chans.push(&Float32Array::from(c.as_slice()));
    }
    Reflect::set(&obj, &"channels".into(), &chans)?;
    Ok(obj.into())
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Decode a recording without creating a project (for a quick look).
#[wasm_bindgen]
pub fn inspect(bytes: &[u8], filename: &str) -> Result<JsValue, JsValue> {
    let ext = filename.rsplit_once('.').map(|(_, e)| e);
    let (audio, info) = decode(bytes, ext).map_err(js_err)?;
    let f = features(&audio, None);
    to_js(
        &serde_json::json!({ "info": info, "features": f, "sha256": nlae_core::hash::sha256(bytes) }),
    )
}

/// Every operation descriptor in the registry.
#[wasm_bindgen]
pub fn descriptors() -> Result<JsValue, JsValue> {
    let d: Vec<_> = nlae_core::ops::registry().descriptors().cloned().collect();
    to_js(&d)
}

/// Analyse decoded audio (planar Float32Arrays) for the analysis worker:
/// `{ features, render_hash }`. The core worker checks the hash matches its
/// source before logging the result.
#[wasm_bindgen]
pub fn analyse_pcm(sample_rate: u32, channels: js_sys::Array) -> Result<JsValue, JsValue> {
    let chans: Vec<Vec<f32>> = channels
        .iter()
        .map(|c| Float32Array::new(&c).to_vec())
        .collect();
    if chans.is_empty() || chans.iter().any(|c| c.len() != chans[0].len()) {
        return Err(js_err("channels must be non-empty and of equal length"));
    }
    let audio = nlae_core::audio::AudioBuffer::new(sample_rate, chans);
    let f = features(&audio, None);
    to_js(&serde_json::json!({ "features": f, "render_hash": audio.render_hash() }))
}

fn selection_of(v: JsValue) -> Result<Option<Selection>, JsValue> {
    if v.is_null() || v.is_undefined() {
        return Ok(None);
    }
    serde_wasm_bindgen::from_value(v).map_err(js_err)
}

/// How a request is handled: `{ route: "recipe" | "steps" | "listen" | "undo" | "model", … }`.
#[wasm_bindgen]
pub fn route(words: &str, selection: JsValue) -> Result<JsValue, JsValue> {
    let sel = selection_of(selection)?;
    to_js(&assistant::route(words, sel.as_ref()))
}

/// Check a model's answer: `{ proposal }` or `{ problems }`.
#[wasm_bindgen]
pub fn parse_response(response: &str) -> Result<JsValue, JsValue> {
    let v: Value = serde_json::from_str(response).map_err(js_err)?;
    match assistant::parse_response(&v) {
        Ok(p) => to_js(&serde_json::json!({ "proposal": p })),
        Err(problems) => to_js(&serde_json::json!({ "problems": problems })),
    }
}

/// A follow-up request asking the model to correct an answer that could not be used.
#[wasm_bindgen]
pub fn correction_request(
    request: &str,
    response: &str,
    problems: JsValue,
) -> Result<String, JsValue> {
    let req: Value = serde_json::from_str(request).map_err(js_err)?;
    let resp: Value = serde_json::from_str(response).map_err(js_err)?;
    let problems: Vec<String> = serde_wasm_bindgen::from_value(problems).map_err(js_err)?;
    Ok(assistant::build_correction(&req, &resp, &problems)
        .map_err(js_err)?
        .to_string())
}

/// Native/WebAssembly parity: resolved-chain hash and render hash, plus the pinned values.
#[wasm_bindgen]
pub fn parity() -> Result<JsValue, JsValue> {
    let (resolved, render) = nlae_core::parity::run();
    to_js(&serde_json::json!({
        "resolved": resolved, "render": render,
        "expected_resolved": nlae_core::parity::EXPECTED_RESOLVED,
        "expected_render": nlae_core::parity::EXPECTED_RENDER,
    }))
}

#[wasm_bindgen]
pub struct WasmProject {
    project: Project<TrackingStore>,
    report: Value,
    /// The open preview's audio, for the preview monitor.
    preview: Option<Preview>,
}

impl WasmProject {
    fn new(project: Project<TrackingStore>, report: Value) -> Self {
        WasmProject {
            project,
            report,
            preview: None,
        }
    }

    fn keep_preview(&mut self, pv: Preview) -> Result<JsValue, JsValue> {
        let out = to_js(&serde_json::json!({ "record": pv.record, "window": pv.record.window }))?;
        self.preview = Some(pv);
        Ok(out)
    }

    /// Borrowed: the recording is not copied for every redraw. The open
    /// preview's audio is `preview:original|before|output|residual`; it covers
    /// only the preview window, which starts at the returned offset (seconds).
    fn audio_at(&mut self, which: &str) -> Result<(&nlae_core::audio::AudioBuffer, f64), JsValue> {
        if let Some(part) = which.strip_prefix("preview:") {
            let pv = self
                .preview
                .as_ref()
                .ok_or_else(|| js_err("no preview is open"))?;
            let a = match part {
                "original" => &pv.original,
                "before" => &pv.before,
                "output" => &pv.output,
                "residual" => &pv.residual,
                other => return Err(js_err(format!("unknown preview audio `{other}`"))),
            };
            return Ok((a, pv.record.window[0]));
        }
        Ok((self.project.audio(which).map_err(js_err)?, 0.0))
    }

    fn audio(&mut self, which: &str) -> Result<&nlae_core::audio::AudioBuffer, JsValue> {
        Ok(self.audio_at(which)?.0)
    }

    fn which(&mut self, which: &str) -> Result<nlae_core::audio::AudioBuffer, JsValue> {
        match which {
            "source" => Ok(self.project.source.clone()),
            "stack" => self.project.current_render().map_err(js_err),
            "residual" => self.project.residual_render().map_err(js_err),
            w => Err(js_err(format!(
                "unknown render `{w}` (source | stack | residual)"
            ))),
        }
    }
}

#[wasm_bindgen]
impl WasmProject {
    /// Import a recording as a new project. `recording` (JSON) marks an in-app
    /// recording and carries the capture settings the browser applied.
    pub fn create(
        bytes: &[u8],
        filename: &str,
        name: Option<String>,
        last_modified: Option<String>,
        recording: Option<String>,
    ) -> Result<WasmProject, JsValue> {
        let recording = match recording {
            Some(r) => Some(serde_json::from_str::<Value>(&r).map_err(js_err)?),
            None => None,
        };
        let opts = CreateOptions {
            name,
            last_modified,
            recording,
            actor: None,
        };
        let p = Project::create(
            TrackingStore::default(),
            &mut WebEnv,
            app(),
            bytes,
            filename,
            opts,
        )
        .map_err(js_err)?;
        Ok(WasmProject::new(
            p,
            serde_json::json!({ "problems": [], "created": true }),
        ))
    }

    /// Open a project from its files (an object of path → Uint8Array).
    pub fn open(files: JsValue) -> Result<WasmProject, JsValue> {
        let obj: Object = files
            .dyn_into()
            .map_err(|_| js_err("files must be an object"))?;
        let mut store = TrackingStore::default();
        for key in Object::keys(&obj).iter() {
            let path = key.as_string().unwrap_or_default();
            let bytes = Uint8Array::new(&Reflect::get(&obj, &key)?).to_vec();
            store.inner.files.insert(path, bytes);
        }
        let (p, report) = Project::open(store, app()).map_err(js_err)?;
        Ok(WasmProject::new(
            p,
            serde_json::to_value(&report).map_err(js_err)?,
        ))
    }

    /// Open a `.nlae` bundle.
    pub fn open_bundle(bytes: &[u8]) -> Result<WasmProject, JsValue> {
        let mem = bundle::unpack(bytes).map_err(js_err)?;
        let store = TrackingStore {
            dirty: mem.files.keys().cloned().collect(),
            inner: mem,
        };
        let (p, report) = Project::open(store, app()).map_err(js_err)?;
        Ok(WasmProject::new(
            p,
            serde_json::to_value(&report).map_err(js_err)?,
        ))
    }

    pub fn id(&self) -> String {
        self.project.id().to_string()
    }

    /// Whether the source still needs analysing (none of the current version logged).
    pub fn needs_analysis(&self) -> bool {
        self.project.needs_analysis()
    }

    /// Log an analysis made by the analysis worker (features as JSON).
    pub fn record_analysis(&mut self, features: &str, render_hash: &str) -> Result<(), JsValue> {
        let f = serde_json::from_str(features).map_err(js_err)?;
        self.project
            .record_analysis(&mut WebEnv, f, render_hash)
            .map_err(js_err)
    }

    /// Why the project is read-only (it did not verify when opened), if it is.
    pub fn read_only(&self) -> Option<String> {
        self.project.read_only().map(str::to_string)
    }

    /// What opening found (source hash, chain, lineage, stack).
    pub fn report(&self) -> Result<JsValue, JsValue> {
        to_js(&self.report)
    }

    pub fn manifest(&self) -> Result<JsValue, JsValue> {
        to_js(&self.project.manifest)
    }

    pub fn state(&self) -> Result<JsValue, JsValue> {
        to_js(self.project.state())
    }

    pub fn events(&self) -> Result<JsValue, JsValue> {
        to_js(&self.project.log.events().to_vec())
    }

    /// Files written since the last call (path → Uint8Array), for persistence.
    /// With `skip_source`, the recording itself is left out (the page stores it
    /// straight from the file it read, without a round trip through here).
    pub fn take_changed_files(&mut self, skip_source: Option<bool>) -> Result<JsValue, JsValue> {
        let skip = skip_source.unwrap_or(false);
        let dirty: Vec<String> = std::mem::take(&mut self.project.store.dirty)
            .into_iter()
            .filter(|p| !(skip && p.starts_with("source/")))
            .collect();
        files_object(&self.project.store.inner, dirty.into_iter())
    }

    /// Every file of the project.
    pub fn all_files(&self) -> Result<JsValue, JsValue> {
        files_object(
            &self.project.store.inner,
            self.project.store.inner.list().into_iter(),
        )
    }

    /// Decoded audio (planar Float32Arrays): `source`, `stack` or `residual`.
    pub fn pcm(&mut self, which: &str) -> Result<JsValue, JsValue> {
        let a = self.audio(which)?;
        planar(a)
    }

    /// Min/max per column over `[t0, t1)` seconds, interleaved.
    pub fn peaks(
        &mut self,
        which: &str,
        t0: f64,
        t1: f64,
        columns: u32,
    ) -> Result<Float32Array, JsValue> {
        let (a, offset) = self.audio_at(which)?;
        let (t0, t1) = (t0 - offset, t1 - offset);
        let cols = columns as usize;
        let dur = a.duration_s();
        let p = if t0 >= 0.0 && t1 <= dur {
            peaks(a, a.time_to_sample(t0), a.time_to_sample(t1), cols)
        } else {
            // The view reaches past the audio (a preview window, zoomed out):
            // columns keep their place in time, and are empty where there is no audio.
            let span = (t1 - t0) / cols.max(1) as f64;
            (0..cols)
                .map(|c| {
                    let (c0, c1) = (t0 + c as f64 * span, t0 + (c + 1) as f64 * span);
                    if c1 <= 0.0 || c0 >= dur {
                        return (0.0, 0.0);
                    }
                    let s0 = a.time_to_sample(c0);
                    peaks(a, s0, a.time_to_sample(c1).max(s0 + 1), 1)[0]
                })
                .collect()
        };
        let flat: Vec<f32> = p.iter().flat_map(|(lo, hi)| [*lo, *hi]).collect();
        Ok(Float32Array::from(flat.as_slice()))
    }

    /// Spectrogram levels (0–255, rows × columns, top row = highest frequency).
    pub fn spectrogram(&mut self, which: &str, request: JsValue) -> Result<Uint8Array, JsValue> {
        let mut req: SpectrogramRequest =
            serde_wasm_bindgen::from_value(request).map_err(js_err)?;
        let (a, offset) = self.audio_at(which)?;
        req.t0 -= offset;
        req.t1 -= offset;
        Ok(Uint8Array::from(draw(a, &req).as_slice()))
    }

    /// Render hash of `source`, `stack` or `residual`: the same hash native
    /// builds record as a step's `output_hash`.
    pub fn render_hash(&mut self, which: &str) -> Result<String, JsValue> {
        Ok(self.audio(which)?.render_hash())
    }

    /// Columns `[c0, c1)` of a spectrogram request (rows × (c1 − c0) levels).
    pub fn spectrogram_part(
        &mut self,
        which: &str,
        request: JsValue,
        c0: u32,
        c1: u32,
    ) -> Result<Uint8Array, JsValue> {
        let mut req: SpectrogramRequest =
            serde_wasm_bindgen::from_value(request).map_err(js_err)?;
        let (a, offset) = self.audio_at(which)?;
        req.t0 -= offset;
        req.t1 -= offset;
        Ok(Uint8Array::from(
            spectrogram_columns(a, &req, c0 as usize, c1 as usize).as_slice(),
        ))
    }

    /// Preview steps the router made from the user's words.
    pub fn preview_routed(&mut self, steps: JsValue, words: &str) -> Result<JsValue, JsValue> {
        let steps: Vec<RoutedStep> = serde_wasm_bindgen::from_value(steps).map_err(js_err)?;
        let drafts = steps
            .iter()
            .map(|s| s.draft(Origin::Console, words))
            .collect();
        let pv = self
            .project
            .preview(&mut WebEnv, drafts, PreviewOptions::default())
            .map_err(js_err)?;
        self.keep_preview(pv)
    }

    /// Replay a built-in recipe on this recording as one plan, and preview it.
    pub fn preview_recipe(&mut self, name: &str, words: &str) -> Result<JsValue, JsValue> {
        let recipe =
            Recipe::builtin(name).ok_or_else(|| js_err(format!("no built-in recipe `{name}`")))?;
        let (_, pv) = nlae_core::recipe::preview_replay(
            &mut self.project,
            &mut WebEnv,
            &recipe,
            ReplayMode::Adaptive,
            Actor::user(),
            None,
            Some(words),
        )
        .map_err(js_err)?;
        self.keep_preview(pv)
    }

    /// The request body for the model (JSON text). The key is not part of it.
    pub fn assistant_request(
        &mut self,
        model: &str,
        history: JsValue,
        words: &str,
        selection: JsValue,
    ) -> Result<String, JsValue> {
        let history: Vec<Turn> = serde_wasm_bindgen::from_value(history).map_err(js_err)?;
        let sel = selection_of(selection)?;
        Ok(
            assistant::request_for(&mut self.project, model, &history, words, sel)
                .map_err(js_err)?
                .to_string(),
        )
    }

    /// Log an exchange with the model; returns its event hash.
    pub fn record_exchange(&mut self, exchange: JsValue) -> Result<String, JsValue> {
        let ex: Exchange = serde_wasm_bindgen::from_value(exchange).map_err(js_err)?;
        assistant::record_exchange(&mut self.project, &mut WebEnv, &ex).map_err(js_err)
    }

    /// Preview what the model proposed, linked to the exchange it came from.
    pub fn preview_proposal(
        &mut self,
        proposal: JsValue,
        words: &str,
        model: &str,
        provider: &str,
        exchange: &str,
    ) -> Result<JsValue, JsValue> {
        let p: Proposal = serde_wasm_bindgen::from_value(proposal).map_err(js_err)?;
        let drafts = p.drafts(&Actor::assistant(model, provider), Origin::Console, words);
        let opts = PreviewOptions {
            plan: p.plan,
            exchange: Some(exchange.to_string()),
            ..Default::default()
        };
        let pv = self
            .project
            .preview(&mut WebEnv, drafts, opts)
            .map_err(js_err)?;
        self.keep_preview(pv)
    }

    /// Accept a preview. `overrides` maps a step's index to changed parameters
    /// (recorded as a modification); `disabled` lists plan steps switched off.
    pub fn accept(
        &mut self,
        preview_id: &str,
        overrides: JsValue,
        disabled: JsValue,
        note: Option<String>,
    ) -> Result<(), JsValue> {
        let raw: BTreeMap<String, Value> =
            serde_wasm_bindgen::from_value(overrides).map_err(js_err)?;
        let mut changes = BTreeMap::new();
        for (k, v) in raw {
            let i: usize = k.parse().map_err(js_err)?;
            changes.insert(i, v);
        }
        let disabled: Vec<usize> = serde_wasm_bindgen::from_value(disabled).map_err(js_err)?;
        let opts = AcceptOptions {
            note,
            overrides: changes,
            disabled,
            actor: None,
        };
        self.project
            .accept(&mut WebEnv, preview_id, opts)
            .map_err(js_err)?;
        self.preview = None;
        Ok(())
    }

    pub fn reject(&mut self, preview_id: &str, reason: Option<String>) -> Result<(), JsValue> {
        self.project
            .reject(&mut WebEnv, preview_id, reason, None)
            .map_err(js_err)?;
        self.preview = None;
        Ok(())
    }

    /// Rate the stack as it stands (1–5), with an optional note.
    pub fn rate_stack(&mut self, overall: u8, note: Option<String>) -> Result<(), JsValue> {
        let target = self.project.state().stack_hash.clone();
        let rating = nlae_core::project::Rating {
            target_kind: "stack".into(),
            target,
            overall,
            dims: Default::default(),
            note,
        };
        self.project.rate(&mut WebEnv, rating, None).map_err(js_err)
    }

    /// Remove the top step (it stays in the log).
    pub fn remove_top(&mut self) -> Result<(), JsValue> {
        self.project.remove_top(&mut WebEnv, None).map_err(js_err)?;
        Ok(())
    }

    /// The stack rendered with the final limiter, as a WAV file (`f32`,
    /// `pcm24` or `pcm16`). The export is logged with its hashes.
    pub fn export_wav(&mut self, format: &str) -> Result<Uint8Array, JsValue> {
        let wav = match format {
            "f32" => WavFormat::F32,
            "pcm24" => WavFormat::Pcm24,
            "pcm16" => WavFormat::Pcm16,
            f => {
                return Err(js_err(format!(
                    "unknown format `{f}` (f32 | pcm24 | pcm16)"
                )))
            }
        };
        let fin = self.project.render_final().map_err(js_err)?;
        let bytes = write_wav(&fin.audio, wav);
        let file = format!("{}.wav", self.project.manifest.project.name);
        self.project
            .record(
                &mut WebEnv,
                "render.exported",
                None,
                serde_json::json!({
                    "file": file,
                    "format": format,
                    "stack_hash": fin.stack_hash,
                    "output_hash": fin.output_hash,
                    "limiter": fin.limiter,
                    "file_sha256": nlae_core::hash::sha256(&bytes),
                }),
            )
            .map_err(js_err)?;
        Ok(Uint8Array::from(bytes.as_slice()))
    }

    pub fn features(&mut self, which: &str) -> Result<JsValue, JsValue> {
        let a = self.which(which)?;
        let f = self.project.features_for(&a);
        to_js(&f)
    }

    /// Store interface state (zoom, selection, …) in the manifest.
    pub fn set_view(&mut self, view: JsValue) -> Result<(), JsValue> {
        let v: Value = serde_wasm_bindgen::from_value(view).map_err(js_err)?;
        self.project.set_view(v).map_err(js_err)
    }

    /// Clone at a step (or the current state). The parent's log records it too.
    pub fn clone_project(
        &mut self,
        at_step: Option<String>,
        name: Option<String>,
    ) -> Result<WasmProject, JsValue> {
        let child = self
            .project
            .clone_into(
                &mut WebEnv,
                TrackingStore::default(),
                at_step.as_deref(),
                name.as_deref(),
                None,
            )
            .map_err(js_err)?;
        Ok(WasmProject::new(
            child,
            serde_json::json!({ "problems": [], "cloned": true }),
        ))
    }

    /// The portable bundle. The export is logged in the project first.
    pub fn export_bundle(&mut self) -> Result<Uint8Array, JsValue> {
        let bytes = self
            .project
            .export_bundle(&mut WebEnv, None)
            .map_err(js_err)?;
        Ok(Uint8Array::from(bytes.as_slice()))
    }

    /// Source files keyed by path, grouped for the library: `{ sha256, filename, path }`.
    pub fn source_ref(&self) -> Result<JsValue, JsValue> {
        to_js(&self.project.manifest.source)
    }
}

/// Counts of the project's files by top-level folder (for diagnostics).
#[wasm_bindgen]
pub fn file_summary(project: &WasmProject) -> Result<JsValue, JsValue> {
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    for f in project.project.store.inner.list() {
        *m.entry(f.split('/').next().unwrap_or("").to_string())
            .or_default() += 1;
    }
    to_js(&m)
}
