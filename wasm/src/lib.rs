//! WebAssembly bindings over the core, used from a Web Worker.
//!
//! A `WasmProject` holds one project in memory. Files the project writes are
//! tracked so the page can persist exactly what changed to the browser's
//! private file storage (OPFS): the source once, the log as it grows, the
//! manifest when it changes.

use std::collections::{BTreeMap, BTreeSet};

use js_sys::{Float32Array, Object, Reflect, Uint8Array};
use nlae_core::analysis::spectrogram::{spectrogram as draw, SpectrogramRequest};
use nlae_core::analysis::{features, peaks::peaks};
use nlae_core::audio::decode;
use nlae_core::project::store::{MemStore, Store, StoreError};
use nlae_core::project::{bundle, CreateOptions, Project};
use nlae_core::provenance::env::{format_rfc3339_ms, ulid};
use nlae_core::provenance::{AppInfo, Env};
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
}

impl WasmProject {
    fn new(project: Project<TrackingStore>, report: Value) -> Self {
        WasmProject { project, report }
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
    pub fn take_changed_files(&mut self) -> Result<JsValue, JsValue> {
        let dirty: Vec<String> = std::mem::take(&mut self.project.store.dirty)
            .into_iter()
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
        let a = self.which(which)?;
        planar(&a)
    }

    /// Min/max per column over `[t0, t1)` seconds, interleaved.
    pub fn peaks(
        &mut self,
        which: &str,
        t0: f64,
        t1: f64,
        columns: u32,
    ) -> Result<Float32Array, JsValue> {
        let a = self.which(which)?;
        let (s0, s1) = (a.time_to_sample(t0), a.time_to_sample(t1));
        let p = peaks(&a, s0, s1, columns as usize);
        let flat: Vec<f32> = p.iter().flat_map(|(lo, hi)| [*lo, *hi]).collect();
        Ok(Float32Array::from(flat.as_slice()))
    }

    /// Spectrogram levels (0–255, rows × columns, top row = highest frequency).
    pub fn spectrogram(&mut self, which: &str, request: JsValue) -> Result<Uint8Array, JsValue> {
        let req: SpectrogramRequest = serde_wasm_bindgen::from_value(request).map_err(js_err)?;
        let a = self.which(which)?;
        Ok(Uint8Array::from(draw(&a, &req).as_slice()))
    }

    /// Render hash of `source`, `stack` or `residual`: the same hash native
    /// builds record as a step's `output_hash`.
    pub fn render_hash(&mut self, which: &str) -> Result<String, JsValue> {
        Ok(self.which(which)?.render_hash())
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
