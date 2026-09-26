//! Projects: one untouched source, one stack of accepted steps, and the
//! hash-chained event log that records everything that happened.
//!
//! Layout (a directory while working, a `.nlae` ZIP when shared):
//! ```text
//! manifest.json               pointers and caches (rewritable, not evidence)
//! source/<original filename>  the original bytes, never modified or renamed
//! events.jsonl                the append-only event log (source of truth)
//! lineage/<ancestor>.jsonl    ancestor logs up to the fork (clones only)
//! analysis/                   derived feature cache
//! renders/                    derived render cache (not bundled)
//! ```

pub mod bindings;
pub mod bundle;
mod cache;
pub mod stack;
pub mod step;
pub mod store;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::analysis::{features, Features, FEATURES_VERSION};
use crate::audio::{decode, AudioBuffer, SourceInfo};
use crate::engine::{self, RenderStep};
use crate::hash::{sha256, ZERO_HASH};
use crate::ops::{registry, OpError};
use crate::provenance::{Actor, AppInfo, Env, EventLog, Verification};
use crate::scope::Scope;
use crate::timeline;
pub use bindings::DiffEntry;
pub use cache::DEFAULT_LIMIT as RENDER_CACHE_LIMIT;
pub use stack::{PreviewRecord, StackChange, StackEntry, StackState};
pub use step::{Binding, BindingSource, Origin, StateSnapshot, Step, StepDraft};
use store::{MemStore, Store, StoreError};

pub const FORMAT: &str = "nlae-project";
pub const FORMAT_VERSION: u32 = 1;
/// Default preview length.
pub const PREVIEW_S: f64 = 10.0;
/// Seconds of audio either side of a time edit in its preview.
pub const EDIT_CONTEXT_S: f64 = 3.0;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Decode(#[from] crate::audio::decode::DecodeError),
    #[error(transparent)]
    Op(#[from] OpError),
    #[error(transparent)]
    Projection(#[from] stack::ProjectionError),
    #[error(transparent)]
    Bundle(#[from] bundle::BundleError),
    #[error("malformed JSON in `{0}`: {1}")]
    Json(String, String),
    #[error("{0}")]
    Invalid(String),
    #[error("this project did not verify and is read-only: {0}")]
    ReadOnly(String),
}

type Result<T> = std::result::Result<T, ProjectError>;

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(ProjectError::Invalid(msg.into()))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SourceRef {
    pub sha256: String,
    pub filename: String,
    pub size: u64,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LogHead {
    pub events: u64,
    pub head: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Lineage {
    pub parent_project: String,
    pub parent_name: String,
    /// Hash of the parent's `project.clone_made` event; the clone's log starts from it.
    pub parent_head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_at_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_at_event: Option<String>,
    /// Ancestor project ids, parent first.
    pub ancestors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub created: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StackSummary {
    pub steps: Vec<String>,
    pub stack_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub project: ProjectInfo,
    pub source: SourceRef,
    pub source_info: SourceInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<Lineage>,
    /// Interface state (zoom, scroll, selection, colour range). Not evidence.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub view: Value,
    pub log: LogHead,
    pub stack: StackSummary,
    /// Parameters of the final limiter (defaults if empty).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub final_limiter: Value,
}

#[derive(Clone, Debug, Default)]
pub struct CreateOptions {
    pub name: Option<String>,
    /// The file's last-modified time, if known (RFC 3339).
    pub last_modified: Option<String>,
    /// For in-app recordings: the capture settings the browser actually applied.
    pub recording: Option<Value>,
    pub actor: Option<Actor>,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewOptions {
    /// Seconds; defaults to 10 s from the first step's scope start (or the clip start).
    pub window: Option<(f64, f64)>,
    /// Group even a single step as a plan.
    pub plan: bool,
    /// The recipe this preview replays, if any.
    pub recipe: Option<Value>,
    /// The `assistant.exchange` event (its hash) the steps came from, if any.
    pub exchange: Option<String>,
    /// The `speech.transcribed` event (its hash), when the request was spoken.
    pub dictation: Option<String>,
}

/// A preview: the record logged, and the audio for listening.
pub struct Preview {
    pub record: PreviewRecord,
    /// The source over the window.
    pub original: AudioBuffer,
    /// The stack before the previewed steps, over the window.
    pub before: AudioBuffer,
    /// With the previewed steps, over the window.
    pub output: AudioBuffer,
    /// What the previewed steps remove: `before − output`.
    pub residual: AudioBuffer,
}

#[derive(Clone, Debug, Default)]
pub struct AcceptOptions {
    pub note: Option<String>,
    /// Parameter changes by step index within the preview (recorded as `step.modified`).
    pub overrides: BTreeMap<usize, Value>,
    /// Steps of a plan switched off before accepting.
    pub disabled: Vec<usize>,
    pub actor: Option<Actor>,
}

/// How a request is applied at once (`Project::apply`).
#[derive(Clone, Debug, Default)]
pub struct ApplyOptions {
    /// Group even a single step as a plan.
    pub plan: bool,
    /// The recipe the steps replay, if any.
    pub recipe: Option<Value>,
    /// The `assistant.exchange` event (its hash) the steps came from, if any.
    pub exchange: Option<String>,
    /// The `speech.transcribed` event (its hash), when the request was spoken.
    pub dictation: Option<String>,
    /// Who applied them (the user, unless said otherwise).
    pub actor: Option<Actor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Rating {
    /// `step`, `plan` or `stack`.
    pub target_kind: String,
    /// Step id, plan id, or stack hash.
    pub target: String,
    /// 1–5.
    pub overall: u8,
    #[serde(default)]
    pub dims: BTreeMap<String, u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LineageCheck {
    pub project: String,
    pub verification: Verification,
}

/// What opening a project found. Problems are reported, never silently repaired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct OpenReport {
    pub source_sha256: String,
    pub source_ok: bool,
    pub log: Verification,
    pub lineage: Vec<LineageCheck>,
    pub projection_ok: bool,
    pub manifest_consistent: bool,
    pub problems: Vec<String>,
}

impl OpenReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub struct FinalRender {
    pub audio: AudioBuffer,
    pub stack_hash: String,
    pub output_hash: String,
    pub limiter: Map<String, Value>,
    /// Where time was removed or silence inserted (seconds; empty without time edits).
    pub edits: Value,
    /// Cue markers for an exported file: output sample and label.
    pub cues: Vec<(u32, String)>,
}

pub struct Project<S: Store> {
    pub store: S,
    pub manifest: Manifest,
    pub log: EventLog,
    pub app: AppInfo,
    pub source: AudioBuffer,
    state: StackState,
    renders: cache::RenderCache,
    features_cache: HashMap<String, Features>,
    /// Persist renders under `renders/` (directory projects).
    pub cache_renders: bool,
    /// Set when opening found problems. Nothing is written to a project that
    /// does not verify: no event is chained onto a broken log.
    read_only: Option<String>,
    /// Render hash of the source, computed when first needed.
    source_render_hash: std::cell::OnceCell<String>,
}

fn to_json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("serialisable")
}

fn parse_json<T: serde::de::DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| ProjectError::Json(what.to_string(), e.to_string()))
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, e)| e)
}

fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

/// Is a file name safe to store as-is? (The original name is kept, never renamed.)
fn check_filename(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return invalid(format!("unusable source file name `{name}`"));
    }
    Ok(())
}

impl<S: Store> Project<S> {
    /// Import a recording (or register an in-app recording) as a new project.
    pub fn create(
        mut store: S,
        env: &mut dyn Env,
        app: AppInfo,
        source_bytes: &[u8],
        filename: &str,
        opts: CreateOptions,
    ) -> Result<Self> {
        check_filename(filename)?;
        let (audio, info) = decode(source_bytes, ext_of(filename))?;
        let sha = sha256(source_bytes);
        let id = env.new_id("pr");
        let name = opts
            .name
            .clone()
            .unwrap_or_else(|| stem_of(filename).to_string());
        let path = format!("source/{filename}");
        store.write_new(&path, source_bytes)?;
        let source = SourceRef {
            sha256: sha.clone(),
            filename: filename.to_string(),
            size: source_bytes.len() as u64,
            path,
        };
        let created = env.now();
        let manifest = Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            project: ProjectInfo {
                id: id.clone(),
                name: name.clone(),
                created,
            },
            source: source.clone(),
            source_info: info.clone(),
            lineage: None,
            view: json!({}),
            log: LogHead {
                events: 0,
                head: ZERO_HASH.into(),
            },
            stack: StackSummary {
                steps: vec![],
                stack_hash: sha.clone(),
            },
            final_limiter: json!({}),
        };
        let mut p = Project {
            store,
            manifest,
            log: EventLog::new_original(),
            app,
            source: audio,
            state: StackState {
                stack_hash: sha,
                ..Default::default()
            },
            renders: cache::RenderCache::default(),
            features_cache: HashMap::new(),
            cache_renders: false,
            read_only: None,
            source_render_hash: std::cell::OnceCell::new(),
        };
        let actor = opts.actor.clone().unwrap_or_else(Actor::user);
        p.append(
            env,
            "project.created",
            &actor,
            json!({ "project_id": id, "name": name }),
        )?;
        let kind = if opts.recording.is_some() {
            "source.recorded"
        } else {
            "source.imported"
        };
        let mut data = json!({ "source": source, "info": info });
        if let Some(lm) = &opts.last_modified {
            data["last_modified"] = json!(lm);
        }
        if let Some(rec) = &opts.recording {
            data["capture"] = rec.clone();
        }
        p.append(env, kind, &actor, data)?;
        Ok(p)
    }

    /// Render hash of the source (the samples, not the file bytes).
    pub fn source_render_hash(&self) -> &str {
        self.source_render_hash
            .get_or_init(|| self.source.render_hash())
    }

    /// Whether the source still needs analysing: no `analysis.computed` event of
    /// the current features version is in the log. The source never changes
    /// (its hash is verified on opening), so such an event always describes it.
    pub fn needs_analysis(&self) -> bool {
        !self.log.events().iter().any(|e| {
            e["type"] == "analysis.computed" && e["data"]["features_version"] == FEATURES_VERSION
        })
    }

    /// Analyse the source and log the result (`analysis.computed`). Creating a
    /// project does not analyse it, so an interface can show the recording
    /// first; the command line calls this straight after `create`.
    pub fn analyse(&mut self, env: &mut dyn Env) -> Result<()> {
        if !self.needs_analysis() {
            return Ok(());
        }
        let hash = self.source_render_hash().to_string();
        let f = match self.features_cache.get(&hash) {
            Some(f) => f.clone(),
            None => features(&self.source, None),
        };
        self.log_analysis(env, f, &hash)
    }

    /// Log an analysis made elsewhere (a background worker in the browser),
    /// after checking it describes this source with the current analysis.
    pub fn record_analysis(
        &mut self,
        env: &mut dyn Env,
        features: Features,
        render_hash: &str,
    ) -> Result<()> {
        if features.version != FEATURES_VERSION {
            return invalid(format!(
                "analysis version {} is not the current version {FEATURES_VERSION}",
                features.version
            ));
        }
        if render_hash != self.source_render_hash() {
            return invalid("the analysis describes different audio from this project's source");
        }
        self.log_analysis(env, features, render_hash)
    }

    fn log_analysis(&mut self, env: &mut dyn Env, f: Features, render_hash: &str) -> Result<()> {
        if !self.needs_analysis() {
            return Ok(());
        }
        self.writable()?;
        let path = format!(
            "analysis/features-v{}-{}.json",
            f.version,
            render_hash.trim_start_matches("sha256:")
        );
        let _ = self
            .store
            .replace(&path, &serde_json::to_vec(&f).expect("serialisable"));
        self.features_cache
            .insert(render_hash.to_string(), f.clone());
        self.append(
            env,
            "analysis.computed",
            &Actor::system(),
            json!({
                "features_version": f.version,
                "render_hash": render_hash,
                "features_hash": sha256(&crate::provenance::jcs::canonical_bytes(&to_json(&f)).expect("finite")),
                "summary": {
                    "duration_s": f.t1 - f.t0,
                    "peak_dbfs": f.peak_dbfs,
                    "loudness_lufs": f.loudness_lufs,
                    "noise_floor_dbfs": f.noise_floor_dbfs,
                    "tonal_lines": f.tonal_lines.len(),
                    "hum_hz": f.hum.as_ref().map(|h| h.fundamental_hz),
                    "clipped_samples": f.clipped_samples,
                },
            }),
        )?;
        Ok(())
    }

    /// Open and verify a project. Problems are reported in the [`OpenReport`].
    pub fn open(store: S, app: AppInfo) -> Result<(Self, OpenReport)> {
        let manifest: Manifest = parse_json(&store.read("manifest.json")?, "manifest.json")?;
        let mut problems = Vec::new();
        if manifest.format != FORMAT || manifest.format_version != FORMAT_VERSION {
            problems.push(format!(
                "unknown project format {} v{}",
                manifest.format, manifest.format_version
            ));
        }
        let bytes = store.read(&manifest.source.path)?;
        let computed = sha256(&bytes);
        let source_ok = computed == manifest.source.sha256;
        if !source_ok {
            problems.push(format!(
                "the source file does not match its recorded hash (recorded {}, found {computed})",
                manifest.source.sha256
            ));
        }
        let (audio, _) = decode(&bytes, ext_of(&manifest.source.filename))?;
        let text = String::from_utf8(store.read("events.jsonl")?)
            .map_err(|_| ProjectError::Invalid("events.jsonl is not UTF-8".into()))?;
        let genesis = manifest
            .lineage
            .as_ref()
            .map(|l| l.parent_head.clone())
            .unwrap_or_else(|| ZERO_HASH.into());
        let (log, verification) = EventLog::parse(&text, Some(&genesis));
        if let Some(f) = &verification.first_failure {
            problems.push(format!(
                "event log broken at line {} (seq {}): {}",
                f.line, f.expected_seq, f.reason
            ));
        }
        let mut lineage = Vec::new();
        if let Some(l) = &manifest.lineage {
            verify_ancestor(
                &store,
                &l.parent_project,
                &l.parent_head,
                &computed,
                &mut lineage,
                &mut problems,
                0,
            );
        }
        let (state, projection_ok) = match stack::project(log.events(), &manifest.source.sha256) {
            Ok(s) => (s, true),
            Err(e) => {
                problems.push(format!("the stack cannot be rebuilt from the log: {e}"));
                (
                    StackState {
                        stack_hash: manifest.source.sha256.clone(),
                        ..Default::default()
                    },
                    false,
                )
            }
        };
        let summary_now = StackSummary {
            steps: state.steps.iter().map(|s| s.step_id.clone()).collect(),
            stack_hash: state.stack_hash.clone(),
        };
        let head_now = LogHead {
            events: log.len() as u64,
            head: log.head_hash(),
        };
        let manifest_consistent = summary_now == manifest.stack && head_now == manifest.log;
        if verification.ok && !manifest_consistent {
            if head_now != manifest.log {
                problems.push(format!(
                    "the manifest records {} events ending {}, the log has {} ending {}",
                    manifest.log.events, manifest.log.head, head_now.events, head_now.head
                ));
            } else {
                problems.push(
                    "the manifest's stack differs from the stack rebuilt from the log".into(),
                );
            }
        }
        let report = OpenReport {
            source_sha256: computed,
            source_ok,
            log: verification,
            lineage,
            projection_ok,
            manifest_consistent,
            problems,
        };
        let read_only = (!report.ok()).then(|| report.problems.join("; "));
        let p = Project {
            store,
            manifest,
            log,
            app,
            source: audio,
            state,
            renders: cache::RenderCache::default(),
            features_cache: HashMap::new(),
            cache_renders: false,
            read_only,
            source_render_hash: std::cell::OnceCell::new(),
        };
        Ok((p, report))
    }

    pub fn id(&self) -> &str {
        &self.manifest.project.id
    }

    pub fn state(&self) -> &StackState {
        &self.state
    }

    /// Why the project is read-only, if it is (it did not verify when opened).
    pub fn read_only(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    pub fn source_sha256(&self) -> &str {
        &self.manifest.source.sha256
    }

    pub fn source_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.store.read(&self.manifest.source.path)?)
    }

    pub fn render_steps(&self) -> Vec<RenderStep> {
        self.state.steps.iter().map(Step::render_step).collect()
    }

    /// Append an event, then persist it and the manifest.
    pub fn append(
        &mut self,
        env: &mut dyn Env,
        kind: &str,
        actor: &Actor,
        data: Value,
    ) -> Result<Value> {
        self.writable()?;
        let ev = self.log.append(env, &self.app, kind, actor, data);
        // An event the stack cannot take is never written.
        let state = match stack::project(self.log.events(), &self.manifest.source.sha256) {
            Ok(s) => s,
            Err(e) => {
                self.log.pop();
                return Err(e.into());
            }
        };
        if let Err(e) = self
            .store
            .append("events.jsonl", EventLog::line_of(&ev).as_bytes())
        {
            self.log.pop();
            return Err(e.into());
        }
        self.state = state;
        self.manifest.log = LogHead {
            events: self.log.len() as u64,
            head: self.log.head_hash(),
        };
        self.manifest.stack = StackSummary {
            steps: self.state.steps.iter().map(|s| s.step_id.clone()).collect(),
            stack_hash: self.state.stack_hash.clone(),
        };
        self.save_manifest()?;
        Ok(ev)
    }

    fn writable(&self) -> Result<()> {
        match &self.read_only {
            Some(why) => Err(ProjectError::ReadOnly(why.clone())),
            None => Ok(()),
        }
    }

    pub fn save_manifest(&mut self) -> Result<()> {
        self.writable()?;
        let bytes = serde_json::to_vec_pretty(&self.manifest).expect("serialisable");
        self.store.replace("manifest.json", &bytes)?;
        Ok(())
    }

    /// Store interface state (zoom, selection, …) in the manifest. Not logged.
    pub fn set_view(&mut self, view: Value) -> Result<()> {
        self.manifest.view = view;
        self.save_manifest()
    }

    /// Full render of the source through `steps` (cached by stack hash), with
    /// the last step's measurements.
    pub fn render_prefix(
        &mut self,
        steps: &[RenderStep],
    ) -> Result<(Arc<AudioBuffer>, Map<String, Value>)> {
        let src_hash = self.manifest.source.sha256.clone();
        let keep = self.state.stack_hash.clone();
        let mut hashes = Vec::with_capacity(steps.len());
        let mut h = src_hash.clone();
        for s in steps {
            h = step::next_stack_hash(&h, s);
            hashes.push(h.clone());
        }
        let target = hashes.last().cloned().unwrap_or(src_hash);
        if let Some(hit) = self.renders.get(&target) {
            return Ok(hit);
        }
        if steps.is_empty() {
            let source = Arc::new(self.source.clone());
            self.renders
                .insert(target, source.clone(), Map::new(), &keep);
            return Ok((source, Map::new()));
        }
        // Start from the longest cached prefix.
        let mut start = 0;
        let mut cur: Option<Arc<AudioBuffer>> = None;
        for i in (0..steps.len()).rev() {
            if let Some((a, _)) = self.renders.get(&hashes[i]) {
                cur = Some(a);
                start = i + 1;
                break;
            }
            if let Some(a) = self.load_render(&hashes[i]) {
                cur = Some(Arc::new(a));
                start = i + 1;
                break;
            }
        }
        let mut meas = Map::new();
        for i in start..steps.len() {
            let out = engine::render_full(
                cur.as_deref().unwrap_or(&self.source),
                std::slice::from_ref(&steps[i]),
            )?;
            let audio = Arc::new(out.audio);
            meas = out.measurements.into_iter().next().unwrap_or_default();
            self.save_render(&hashes[i], &audio);
            self.renders
                .insert(hashes[i].clone(), audio.clone(), meas.clone(), &keep);
            cur = Some(audio);
        }
        let cur = cur.expect("rendered or cached");
        if start == steps.len() {
            // Loaded from disk, where measurements are not kept.
            self.renders
                .insert(target, cur.clone(), meas.clone(), &keep);
        }
        Ok((cur, meas))
    }

    /// Change how many bytes of renders are kept in memory.
    pub fn set_render_cache_limit(&mut self, bytes: usize) {
        self.renders.limit = bytes;
    }

    fn render_path(hash: &str) -> String {
        format!("renders/{}.f32", hash.trim_start_matches("sha256:"))
    }

    fn save_render(&mut self, hash: &str, audio: &AudioBuffer) {
        if !self.cache_renders {
            return;
        }
        let mut bytes = Vec::with_capacity(audio.len() * audio.num_channels() * 4 + 32);
        bytes.extend_from_slice(b"nlae-render-v1\0\0");
        bytes.extend_from_slice(&audio.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(audio.num_channels() as u32).to_le_bytes());
        bytes.extend_from_slice(&(audio.len() as u64).to_le_bytes());
        for ch in &audio.channels {
            for x in ch {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        let _ = self.store.replace(&Self::render_path(hash), &bytes);
    }

    fn load_render(&self, hash: &str) -> Option<AudioBuffer> {
        if !self.cache_renders {
            return None;
        }
        let b = self.store.read(&Self::render_path(hash)).ok()?;
        if b.len() < 32 || &b[..16] != b"nlae-render-v1\0\0" {
            return None;
        }
        let sr = u32::from_le_bytes(b[16..20].try_into().ok()?);
        let nch = u32::from_le_bytes(b[20..24].try_into().ok()?) as usize;
        let len = u64::from_le_bytes(b[24..32].try_into().ok()?) as usize;
        if b.len() != 32 + nch * len * 4 || nch == 0 {
            return None;
        }
        let channels = (0..nch)
            .map(|c| {
                (0..len)
                    .map(|i| {
                        f32::from_le_bytes(b[32 + (c * len + i) * 4..][..4].try_into().unwrap())
                    })
                    .collect()
            })
            .collect();
        let audio = AudioBuffer::new(sr, channels);
        // A cache entry is trusted only if it matches the render hash the log recorded.
        let recorded = self
            .state
            .steps
            .iter()
            .find(|s| s.stack_hash.as_deref() == Some(hash))
            .and_then(|s| s.output_hash.clone());
        match recorded {
            Some(h) if h == audio.render_hash() => Some(audio),
            _ => None,
        }
    }

    /// The current stack's output (without the final limiter).
    pub fn current_render(&mut self) -> Result<AudioBuffer> {
        let steps = self.render_steps();
        Ok(Arc::unwrap_or_clone(self.render_prefix(&steps)?.0))
    }

    /// The source, the stack's render or the level-matched residual, borrowed.
    /// Renders are cached by stack hash, so asking again costs nothing (an
    /// interface asks for every redraw).
    pub fn audio(&mut self, which: &str) -> Result<&AudioBuffer> {
        match which {
            "source" => Ok(&self.source),
            "stack" => {
                let steps = self.render_steps();
                if steps.is_empty() {
                    return Ok(&self.source);
                }
                let key = step::stack_hash(&self.manifest.source.sha256, &steps);
                if !self.renders.contains(&key) {
                    self.render_prefix(&steps)?;
                }
                Ok(self.renders.audio(&key).expect("just rendered"))
            }
            "output" => {
                let layout = self.edit_layout();
                if layout.is_identity() {
                    return self.audio("stack");
                }
                let key = format!("output:{}", self.state.stack_hash);
                if !self.renders.contains(&key) {
                    let out = layout.apply(self.audio("stack")?);
                    let keep = self.state.stack_hash.clone();
                    self.renders
                        .insert(key.clone(), Arc::new(out), Map::new(), &keep);
                }
                Ok(self.renders.audio(&key).expect("just made"))
            }
            "residual" => {
                let key = format!("residual:{}", self.state.stack_hash);
                if !self.renders.contains(&key) {
                    let r = self.residual_render()?;
                    let keep = self.state.stack_hash.clone();
                    self.renders
                        .insert(key.clone(), Arc::new(r), Map::new(), &keep);
                }
                Ok(self.renders.audio(&key).expect("just made"))
            }
            w => invalid(format!(
                "unknown render `{w}` (source | stack | output | residual)"
            )),
        }
    }

    /// One active step on its own, over the whole recording: the audio
    /// `before` it (the active steps below it), `after` it, or what it
    /// `removed` (before − after). Cached like the stack's renders.
    pub fn step_audio(&mut self, id: &str, part: &str) -> Result<&AudioBuffer> {
        let i = self
            .state
            .steps
            .iter()
            .position(|s| s.step_id == id)
            .ok_or_else(|| ProjectError::Invalid(format!("no active step `{id}`")))?;
        let steps: Vec<RenderStep> = self.state.steps[..=i]
            .iter()
            .map(Step::render_step)
            .collect();
        let key = match part {
            "before" => step::stack_hash(&self.manifest.source.sha256, &steps[..i]),
            "after" => step::stack_hash(&self.manifest.source.sha256, &steps),
            "removed" => format!(
                "removed:{}",
                step::stack_hash(&self.manifest.source.sha256, &steps)
            ),
            p => return invalid(format!("unknown part `{p}` (before | after | removed)")),
        };
        if !self.renders.contains(&key) {
            match part {
                "before" => {
                    self.render_prefix(&steps[..i])?;
                }
                "after" => {
                    self.render_prefix(&steps)?;
                }
                _ => {
                    let (before, _) = self.render_prefix(&steps[..i])?;
                    let (after, _) = self.render_prefix(&steps)?;
                    let removed = engine::difference(&before, &after);
                    let keep = self.state.stack_hash.clone();
                    self.renders
                        .insert(key.clone(), Arc::new(removed), Map::new(), &keep);
                }
            }
        }
        Ok(self.renders.audio(&key).expect("just rendered"))
    }

    /// What the stack removed, level-matched: the source passed through the
    /// stack's level steps only (DC removal, gain, normalise, compressor) minus
    /// the stack's output. With no attenuating steps it is silence; otherwise it
    /// is exactly what those steps took out, at the level it would have had.
    pub fn residual_render(&mut self) -> Result<AudioBuffer> {
        let reg = registry();
        let all = self.render_steps();
        let level: Vec<RenderStep> = all
            .iter()
            .filter(|s| {
                reg.get(&s.op, s.op_version)
                    .map(|o| o.descriptor().class != crate::ops::OpClass::Attenuative)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        let (matched, _) = self.render_prefix(&level)?;
        let (out, _) = self.render_prefix(&all)?;
        Ok(engine::difference(&matched, &out))
    }

    /// Features of a render (cached by render hash, in memory and under `analysis/`).
    pub fn features_for(&mut self, audio: &AudioBuffer) -> Features {
        let h = audio.render_hash();
        if let Some(f) = self.features_cache.get(&h) {
            return f.clone();
        }
        let path = format!(
            "analysis/features-v{}-{}.json",
            FEATURES_VERSION,
            h.trim_start_matches("sha256:")
        );
        if let Ok(bytes) = self.store.read(&path) {
            if let Ok(f) = serde_json::from_slice::<Features>(&bytes) {
                self.features_cache.insert(h, f.clone());
                return f;
            }
        }
        let f = features(audio, None);
        let _ = self
            .store
            .replace(&path, &serde_json::to_vec(&f).expect("serialisable"));
        self.features_cache.insert(h, f.clone());
        f
    }

    fn snapshot(&mut self, audio: &AudioBuffer, scope: &Scope) -> StateSnapshot {
        let clip = self.features_for(audio);
        let scope_f = scope.time().map(|(t0, t1)| features(audio, Some((t0, t1))));
        StateSnapshot {
            clip,
            scope: scope_f,
        }
    }

    fn default_window(&self, drafts: &[StepDraft]) -> (f64, f64) {
        let dur = self.source.duration_s();
        // A time edit: its join, with some of the audio either side.
        if let Some(d) = drafts.first().filter(|d| timeline::is_edit(&d.op)) {
            let (t0, t1) = d
                .scope
                .time()
                .or_else(|| d.params["at_s"].as_f64().map(|t| (t, t)))
                .unwrap_or((0.0, 0.0));
            return (
                (t0 - EDIT_CONTEXT_S).max(0.0),
                (t1 + EDIT_CONTEXT_S).min(dur),
            );
        }
        let t0 = drafts
            .first()
            .and_then(|d| d.scope.time())
            .map(|t| t.0)
            .unwrap_or(0.0)
            .clamp(0.0, dur);
        let t0 = if dur - t0 < PREVIEW_S {
            (dur - PREVIEW_S).max(0.0)
        } else {
            t0
        };
        (t0, (t0 + PREVIEW_S).min(dur))
    }

    /// Resolve drafts on top of `base`, returning the candidate steps.
    fn resolve_drafts(
        &mut self,
        env: &mut dyn Env,
        base: &[RenderStep],
        drafts: &[StepDraft],
        plan_id: Option<String>,
    ) -> Result<Vec<Step>> {
        let reg = registry();
        let mut chain = base.to_vec();
        let mut out = Vec::new();
        let dur = self.source.duration_s();
        for d in drafts {
            let op = match d.op_version {
                Some(v) => reg.get(&d.op, v)?,
                None => reg.latest(&d.op)?,
            };
            let desc = op.descriptor();
            if desc.system {
                return invalid(format!(
                    "`{}` is added by the application itself and cannot be a step",
                    desc.id
                ));
            }
            let params = reg.validate(&desc.id, desc.version, &d.params, &d.scope)?;
            d.scope
                .validate(dur, self.source.sample_rate)
                .map_err(OpError::from)?;
            let (input, _) = self.render_prefix(&chain)?;
            let resolved = match &d.resolved {
                Some(r) => r.clone(),
                None => op.resolve(&params, &d.scope, &input)?,
            };
            let state_before = self.snapshot(&input, &d.scope);
            let s = Step {
                step_id: env.new_id("st"),
                plan_id: plan_id.clone(),
                op: desc.id.clone(),
                op_version: desc.version,
                params,
                resolved,
                scope: d.scope.clone(),
                bindings: d.bindings.clone(),
                actor: d.actor.clone(),
                origin: d.origin,
                intent: d.intent.clone(),
                rationale: d.rationale.clone(),
                note: d.note.clone(),
                class: desc.class,
                label: desc.class.label().into(),
                state_before: Some(state_before),
                state_after: None,
                measurements: Map::new(),
                input_hash: input.render_hash(),
                output_hash: None,
                stack_hash: None,
                resolved_on: None,
                inherited_from: None,
            };
            chain.push(s.render_step());
            out.push(s);
        }
        Ok(out)
    }

    /// A spoken request named by a preview or an application must be in the log.
    fn check_dictation(&self, dictation: Option<&str>) -> Result<()> {
        if let Some(h) = dictation {
            let logged = self
                .log
                .events()
                .iter()
                .any(|e| e["type"] == "speech.transcribed" && e["hash"] == h);
            if !logged {
                return invalid(format!("no spoken request {h} in this project's log"));
            }
        }
        Ok(())
    }

    /// Render and measure resolved steps on top of `base` (whose stack hash is
    /// `h`): measurements over the whole clip, render and stack hashes, the
    /// features after, inferred bindings.
    fn finalise(&mut self, base: &[RenderStep], h: &str, steps: &mut [Step]) -> Result<()> {
        let mut chain = base.to_vec();
        let mut h = h.to_string();
        for s in steps.iter_mut() {
            let (input, _) = self.render_prefix(&chain)?;
            chain.push(s.render_step());
            let (output, meas) = self.render_prefix(&chain)?;
            h = step::next_stack_hash(&h, &s.render_step());
            s.measurements = meas;
            if timeline::is_edit(&s.op) {
                s.measurements.insert(
                    "output_duration_s".into(),
                    json!(self.output_duration_s(&chain)),
                );
            }
            s.output_hash = Some(output.render_hash());
            s.stack_hash = Some(h.clone());
            s.state_after = Some(self.snapshot(&output, &s.scope));
            if let Some(before) = s.state_before.clone() {
                s.bindings = bindings::infer(s, &before.clip, &input);
            }
        }
        Ok(())
    }

    /// Preview one step, or several as a plan, on a short window. Logged as
    /// `step.previewed`; nothing is committed.
    pub fn preview(
        &mut self,
        env: &mut dyn Env,
        drafts: Vec<StepDraft>,
        opts: PreviewOptions,
    ) -> Result<Preview> {
        if drafts.is_empty() {
            return invalid("nothing to preview");
        }
        self.check_dictation(opts.dictation.as_deref())?;
        let base = self.render_steps();
        let base_hash = self.state.stack_hash.clone();
        let is_plan = opts.plan || drafts.len() > 1;
        let plan_id = if is_plan {
            Some(env.new_id("pl"))
        } else {
            None
        };
        let edit = drafts.iter().any(|d| timeline::is_edit(&d.op));
        if edit && drafts.len() > 1 {
            return invalid("a time edit is previewed on its own, not in a plan");
        }
        let mut steps = self.resolve_drafts(env, &base, &drafts, plan_id.clone())?;
        let dur = self.source.duration_s();
        let (t0, t1) = opts.window.unwrap_or_else(|| self.default_window(&drafts));
        let (t0, t1) = (t0.clamp(0.0, dur), t1.clamp(0.0, dur));
        if t1 <= t0 {
            return invalid(format!("empty preview window {t0}–{t1} s"));
        }
        let (a, b) = (
            self.source.time_to_sample(t0) as i64,
            self.source.time_to_sample(t1) as i64,
        );
        let (before, output, residual) = if edit {
            // Processing is unchanged, so the window comes from the stack's
            // (cached) render; the edit is applied to it, and the residual is
            // what it leaves out, where it was.
            let full = self.render_prefix(&base)?.0;
            let before = full.extract_padded(a, b);
            let mut chain = base.clone();
            chain.push(steps[0].render_step());
            let edits = timeline::edits(&[steps[0].render_step()]);
            let local: Vec<timeline::Edit> = edits
                .iter()
                .map(|e| match *e {
                    timeline::Edit::Remove { s0, s1, fade } => timeline::Edit::Remove {
                        s0: (s0 as i64 - a).max(0) as usize,
                        s1: (s1 as i64 - a).max(0) as usize,
                        fade,
                    },
                    timeline::Edit::Insert { at, len, fade } => timeline::Edit::Insert {
                        at: (at as i64 - a).max(0) as usize,
                        len,
                        fade,
                    },
                })
                .collect();
            let output = timeline::layout(&local, before.len()).apply(&before);
            let mut residual =
                AudioBuffer::silent(before.sample_rate, before.num_channels(), before.len());
            if let Some(timeline::Edit::Remove { s0, s1, .. }) = local.first().copied() {
                let (s0, s1) = (s0.min(before.len()), s1.min(before.len()));
                for (r, b) in residual.channels.iter_mut().zip(&before.channels) {
                    r[s0..s1].copy_from_slice(&b[s0..s1]);
                }
            }
            let s = &mut steps[0];
            let key = if s.op == timeline::REMOVE {
                "removed_s"
            } else {
                "inserted_s"
            };
            s.measurements.insert(key.into(), s.resolved[key].clone());
            s.measurements.insert(
                "output_duration_s".into(),
                json!(self.output_duration_s(&chain)),
            );
            (before, output, residual)
        } else {
            let before = engine::render_window(&self.source, &base, a, b)?;
            let mut chain = base.clone();
            let mut output = before.clone();
            for s in steps.iter_mut() {
                chain.push(s.render_step());
                let (w, m) = engine::render_window_measured(&self.source, &chain, a, b)?;
                s.measurements = m;
                output = w;
            }
            let residual = engine::difference(&before, &output);
            (before, output, residual)
        };
        let record = PreviewRecord {
            preview_id: env.new_id("pv"),
            kind: if is_plan {
                "plan".into()
            } else {
                "step".into()
            },
            plan_id,
            steps,
            window: [crate::math::round_to(t0, 6), crate::math::round_to(t1, 6)],
            base_stack_hash: base_hash,
            recipe: opts.recipe.clone(),
            exchange: opts.exchange.clone(),
            dictation: opts.dictation.clone(),
            seq: 0,
            event_hash: String::new(),
        };
        let actor = drafts[0].actor.clone();
        let mut data = to_json(&record);
        if let Some(m) = data.as_object_mut() {
            m.remove("seq");
            m.remove("event_hash");
        }
        self.append(env, "step.previewed", &actor, data)?;
        let record = self
            .state
            .previews
            .get(&record.preview_id)
            .cloned()
            .expect("just logged");
        Ok(Preview {
            record,
            original: self.source.extract_padded(a, b),
            before,
            output,
            residual,
        })
    }

    /// Accept a preview: full renders, measurements over the whole scope,
    /// features after, inferred bindings; logged as `step.accepted` or `plan.accepted`.
    pub fn accept(
        &mut self,
        env: &mut dyn Env,
        preview_id: &str,
        opts: AcceptOptions,
    ) -> Result<Vec<Step>> {
        let rec = self
            .state
            .previews
            .get(preview_id)
            .cloned()
            .ok_or_else(|| ProjectError::Invalid(format!("no preview `{preview_id}`")))?;
        if let Some(d) = self.state.decided.get(preview_id) {
            return invalid(format!("preview `{preview_id}` was already {d}"));
        }
        if rec.base_stack_hash != self.state.stack_hash {
            return invalid("the stack has changed since this preview; preview again");
        }
        let actor = opts.actor.clone().unwrap_or_else(Actor::user);
        let base = self.render_steps();
        let changed = !opts.overrides.is_empty() || !opts.disabled.is_empty();
        let mut steps: Vec<Step> = if changed {
            // Rebuild the drafts with the changes, log each modification, re-resolve.
            let mut drafts = Vec::new();
            for (i, s) in rec.steps.iter().enumerate() {
                if opts.disabled.contains(&i) {
                    continue;
                }
                let mut d = StepDraft {
                    op: s.op.clone(),
                    op_version: Some(s.op_version),
                    params: s.params.clone(),
                    scope: s.scope.clone(),
                    actor: s.actor.clone(),
                    origin: s.origin,
                    intent: s.intent.clone(),
                    rationale: s.rationale.clone(),
                    note: s.note.clone(),
                    bindings: s.bindings.clone(),
                    resolved: None,
                };
                if let Some(ov) = opts.overrides.get(&i) {
                    let mut p = s.params.clone();
                    if let (Some(pm), Some(om)) = (p.as_object_mut(), ov.as_object()) {
                        for (k, v) in om {
                            pm.insert(k.clone(), v.clone());
                        }
                    }
                    self.append(
                        env,
                        "step.modified",
                        &actor,
                        json!({ "preview_id": preview_id, "index": i, "step_id": s.step_id, "from_params": s.params, "to_params": p }),
                    )?;
                    d.params = p;
                }
                drafts.push(d);
            }
            if drafts.is_empty() {
                return invalid("every step of the plan is switched off");
            }
            self.resolve_drafts(env, &base, &drafts, rec.plan_id.clone())?
        } else {
            rec.steps.clone()
        };
        let top = self.state.stack_hash.clone();
        self.finalise(&base, &top, &mut steps)?;
        if opts.note.is_some() {
            for s in steps.iter_mut() {
                s.note = opts.note.clone();
            }
        }
        if rec.kind == "plan" {
            self.append(
                env,
                "plan.accepted",
                &actor,
                json!({ "preview_id": preview_id, "plan_id": rec.plan_id, "steps": steps, "disabled": opts.disabled }),
            )?;
        } else {
            self.append(
                env,
                "step.accepted",
                &actor,
                json!({ "preview_id": preview_id, "step": steps[0] }),
            )?;
        }
        Ok(self.as_recorded(&steps))
    }

    pub fn reject(
        &mut self,
        env: &mut dyn Env,
        preview_id: &str,
        reason: Option<String>,
        actor: Option<Actor>,
    ) -> Result<()> {
        if !self.state.previews.contains_key(preview_id) {
            return invalid(format!("no preview `{preview_id}`"));
        }
        if let Some(d) = self.state.decided.get(preview_id) {
            return invalid(format!("preview `{preview_id}` was already {d}"));
        }
        self.append(
            env,
            "step.rejected",
            &actor.unwrap_or_else(Actor::user),
            json!({ "preview_id": preview_id, "reason": reason }),
        )?;
        Ok(())
    }

    /// Apply one step, or several as a plan, at once: resolved on the current
    /// stack, rendered over the whole clip and measured. Logged as
    /// `step.applied`; the steps can be removed, restored and edited afterwards.
    pub fn apply(
        &mut self,
        env: &mut dyn Env,
        drafts: Vec<StepDraft>,
        opts: ApplyOptions,
    ) -> Result<Vec<Step>> {
        self.writable()?;
        if drafts.is_empty() {
            return invalid("nothing to apply");
        }
        self.check_dictation(opts.dictation.as_deref())?;
        let base = self.render_steps();
        let is_plan = opts.plan || drafts.len() > 1;
        let plan_id = if is_plan {
            Some(env.new_id("pl"))
        } else {
            None
        };
        let mut steps = self.resolve_drafts(env, &base, &drafts, plan_id.clone())?;
        let top = self.state.stack_hash.clone();
        self.finalise(&base, &top, &mut steps)?;
        let mut data = json!({ "kind": if is_plan { "plan" } else { "step" }, "steps": steps });
        let linked = [
            ("plan_id", plan_id.map(Value::from)),
            ("recipe", opts.recipe),
            ("exchange", opts.exchange.map(Value::from)),
            ("dictation", opts.dictation.map(Value::from)),
        ];
        for (k, v) in linked {
            if let Some(v) = v {
                data[k] = v;
            }
        }
        let actor = opts.actor.unwrap_or_else(Actor::user);
        self.append(env, "step.applied", &actor, data)?;
        Ok(self.as_recorded(&steps))
    }

    /// Exclude steps: they stop playing a part in the render but keep their
    /// places, and can be restored. The steps above keep their values and are
    /// recorded again. Returns them.
    pub fn exclude(
        &mut self,
        env: &mut dyn Env,
        ids: &[String],
        reason: Option<String>,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        self.set_active(env, ids, false, reason, None, actor)
    }

    /// Bring excluded steps back to their places, with the values they had.
    /// The steps above keep their values and are recorded again. Returns the
    /// restored steps and the steps above, as recorded.
    pub fn restore(
        &mut self,
        env: &mut dyn Env,
        ids: &[String],
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        self.set_active(env, ids, true, None, None, actor)
    }

    /// Exclude the top step (it keeps its place and can be restored).
    pub fn remove_top(&mut self, env: &mut dyn Env, actor: Option<Actor>) -> Result<Step> {
        let top = self
            .state
            .steps
            .last()
            .cloned()
            .ok_or_else(|| ProjectError::Invalid("the stack is empty".into()))?;
        self.exclude(env, std::slice::from_ref(&top.step_id), None, actor)?;
        Ok(top)
    }

    /// Change a step's parameters: `changes` are merged into them, and the step
    /// is resolved again on its input. The steps above keep their values and are
    /// recorded again. Logged as `step.edited`; returns the edited step and the
    /// steps above, as recorded.
    pub fn edit(
        &mut self,
        env: &mut dyn Env,
        step_id: &str,
        changes: &Value,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        let cur = self.active_step(step_id)?;
        let mut params = cur.params.clone();
        match (params.as_object_mut(), changes.as_object()) {
            (Some(p), Some(c)) => {
                for (k, v) in c {
                    p.insert(k.clone(), v.clone());
                }
            }
            _ => return invalid("changes are an object of parameter values"),
        }
        let params = registry().validate(&cur.op, cur.op_version, &params, &cur.scope)?;
        // Compared as written: 150 and 150.0 are the same value.
        let canonical = |v: &Value| crate::provenance::jcs::canonical_bytes(v).ok();
        if canonical(&params) == canonical(&cur.params) {
            return invalid(format!("the parameters of `{step_id}` are unchanged"));
        }
        self.resolve_again(env, cur, params, false, actor)
    }

    /// Measure a step again on its current input, with the same parameters
    /// (after a change below it). Logged as `step.edited` with `remeasured`.
    pub fn remeasure(
        &mut self,
        env: &mut dyn Env,
        step_id: &str,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        let cur = self.active_step(step_id)?;
        let params = cur.params.clone();
        self.resolve_again(env, cur, params, true, actor)
    }

    /// What measuring a step again would change. Nothing is changed or logged.
    pub fn remeasure_diff(&mut self, step_id: &str) -> Result<Vec<DiffEntry>> {
        let cur = self.active_step(step_id)?;
        let i = self
            .state
            .steps
            .iter()
            .position(|s| s.step_id == step_id)
            .expect("active");
        let below: Vec<RenderStep> = self.state.steps[..i]
            .iter()
            .map(Step::render_step)
            .collect();
        let (input, _) = self.render_prefix(&below)?;
        let op = registry().get(&cur.op, cur.op_version)?;
        let r = op.resolve(&cur.params, &cur.scope, &input)?;
        let mut diff =
            crate::recipe::resolved_diff(i, &cur.op, Some(&cur.resolved), &r, &cur.params);
        for d in diff.iter_mut() {
            d.reason = "measured again on the audio below it now".into();
        }
        Ok(diff)
    }

    /// Take back the last change to the stack, logged as the inverse change
    /// (`undoes`). Returns the change taken back.
    pub fn undo(&mut self, env: &mut dyn Env, actor: Option<Actor>) -> Result<StackChange> {
        let c = self
            .state
            .undo
            .last()
            .cloned()
            .ok_or_else(|| ProjectError::Invalid("nothing to undo".into()))?;
        let link = Some(("undoes", c.event.clone()));
        match c.kind.as_str() {
            "applied" | "restored" => {
                self.set_active(env, &c.step_ids, false, None, link, actor)?;
            }
            "excluded" => {
                self.set_active(env, &c.step_ids, true, None, link, actor)?;
            }
            _ => {
                let before = c.before.as_deref().unwrap_or_default();
                let version = self.recorded_version(before, &c.step_ids[0])?;
                self.edit_to(env, version, link, actor)?;
            }
        }
        Ok(c)
    }

    /// Repeat the last change taken back, logged as that change again
    /// (`redoes`). Returns the change repeated.
    pub fn redo(&mut self, env: &mut dyn Env, actor: Option<Actor>) -> Result<StackChange> {
        let c = self
            .state
            .redo
            .last()
            .cloned()
            .ok_or_else(|| ProjectError::Invalid("nothing to redo".into()))?;
        let link = Some(("redoes", c.event.clone()));
        match c.kind.as_str() {
            "applied" | "restored" => {
                self.set_active(env, &c.step_ids, true, None, link, actor)?;
            }
            "excluded" => {
                self.set_active(env, &c.step_ids, false, None, link, actor)?;
            }
            _ => {
                let version = self.recorded_version(&c.event, &c.step_ids[0])?;
                self.edit_to(env, version, link, actor)?;
            }
        }
        Ok(c)
    }

    /// An active step, by id.
    fn active_step(&self, id: &str) -> Result<Step> {
        if let Some(s) = self.state.steps.iter().find(|s| s.step_id == id) {
            return Ok(s.clone());
        }
        if self.state.excluded.contains_key(id) {
            return invalid(format!("`{id}` is excluded; restore it first"));
        }
        invalid(format!("no step `{id}` on the stack"))
    }

    /// A step as an event in this log recorded it.
    fn recorded_version(&self, event: &str, id: &str) -> Result<Step> {
        self.log
            .events()
            .iter()
            .find(|e| e["hash"] == event)
            .and_then(|e| stack::step_in_event(e, id))
            .ok_or_else(|| {
                ProjectError::Invalid(format!("no version of `{id}` recorded by {event}"))
            })
    }

    /// Every place in the stack: each step in its current version, and whether
    /// it is active.
    fn places(&self) -> Vec<(Step, bool)> {
        self.state
            .entries
            .iter()
            .map(|e| {
                let s = self.state.step(&e.step_id).expect("projected").clone();
                (s, e.active)
            })
            .collect()
    }

    fn set_active(
        &mut self,
        env: &mut dyn Env,
        ids: &[String],
        on: bool,
        reason: Option<String>,
        link: Option<(&str, String)>,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        self.writable()?;
        if ids.is_empty() {
            return invalid("no steps named");
        }
        let mut places = self.places();
        let mut k = usize::MAX;
        for (n, id) in ids.iter().enumerate() {
            if ids[..n].contains(id) {
                return invalid(format!("`{id}` is named twice"));
            }
            let i = self
                .state
                .position(id)
                .ok_or_else(|| ProjectError::Invalid(format!("no step `{id}` on the stack")))?;
            if places[i].1 == on {
                return invalid(format!(
                    "`{id}` is already {}",
                    if on { "on the stack" } else { "excluded" }
                ));
            }
            places[i].1 = on;
            k = k.min(i);
        }
        let chain = self.record_again(&places, k, None)?;
        let mut data = json!({ "step_ids": ids, "chain": chain });
        if let Some(r) = reason {
            data["reason"] = json!(r);
        }
        if let Some((key, h)) = link {
            data[key] = json!(h);
        }
        let kind = if on { "step.restored" } else { "step.excluded" };
        self.append(env, kind, &actor.unwrap_or_else(Actor::user), data)?;
        Ok(self.as_recorded(&chain))
    }

    /// Resolve an active step again on its input with `params`, and record it
    /// and the steps above it.
    fn resolve_again(
        &mut self,
        env: &mut dyn Env,
        cur: Step,
        params: Value,
        remeasured: bool,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        self.writable()?;
        let i = self
            .state
            .steps
            .iter()
            .position(|s| s.step_id == cur.step_id)
            .expect("active");
        let below: Vec<RenderStep> = self.state.steps[..i]
            .iter()
            .map(Step::render_step)
            .collect();
        let (input, _) = self.render_prefix(&below)?;
        let op = registry().get(&cur.op, cur.op_version)?;
        let mut new = cur.clone();
        new.resolved = op.resolve(&params, &cur.scope, &input)?;
        new.params = params;
        new.resolved_on = None;
        self.log_edit(env, cur, new, remeasured, None, actor)
    }

    /// Bring back a recorded version of a step (undoing or redoing an edit).
    fn edit_to(
        &mut self,
        env: &mut dyn Env,
        version: Step,
        link: Option<(&str, String)>,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        self.writable()?;
        let cur = self.active_step(&version.step_id)?;
        let mut new = cur.clone();
        new.params = version.params.clone();
        new.resolved = version.resolved.clone();
        // Where its values were measured; compared with its input when recorded.
        new.resolved_on = Some(version.resolved_on.unwrap_or(version.input_hash));
        self.log_edit(env, cur, new, false, link, actor)
    }

    fn log_edit(
        &mut self,
        env: &mut dyn Env,
        cur: Step,
        new: Step,
        remeasured: bool,
        link: Option<(&str, String)>,
        actor: Option<Actor>,
    ) -> Result<Vec<Step>> {
        let k = self.state.position(&cur.step_id).expect("on the stack");
        let mut places = self.places();
        let to_params = new.params.clone();
        places[k].0 = new;
        let chain = self.record_again(&places, k, Some(&cur.step_id))?;
        let mut data = json!({
            "step_id": cur.step_id,
            "from_params": cur.params,
            "to_params": to_params,
            "chain": chain,
        });
        if remeasured {
            data["remeasured"] = json!(true);
        }
        if let Some((key, h)) = link {
            data[key] = json!(h);
        }
        self.append(env, "step.edited", &actor.unwrap_or_else(Actor::user), data)?;
        Ok(self.as_recorded(&chain))
    }

    /// Steps as the log now records them (numbers as they read back).
    fn as_recorded(&self, steps: &[Step]) -> Vec<Step> {
        steps
            .iter()
            .map(|s| {
                self.state
                    .step(&s.step_id)
                    .cloned()
                    .unwrap_or_else(|| s.clone())
            })
            .collect()
    }

    /// New versions of the active steps from place `k` of `places` up, with
    /// `places` as the stack will be. Each step keeps its values; what depends
    /// on its input (measurements, snapshots, inferred bindings, render and
    /// stack hashes) is recorded again. A step whose input and values have not
    /// changed has the same output, so it is not rendered again. `fresh` names a
    /// step whose values changed (an edit), with `resolved_on` saying where they
    /// were measured if not on its current input.
    fn record_again(
        &mut self,
        places: &[(Step, bool)],
        k: usize,
        fresh: Option<&str>,
    ) -> Result<Vec<Step>> {
        let reg = registry();
        let below: Vec<&Step> = places[..k]
            .iter()
            .filter(|(_, on)| *on)
            .map(|(s, _)| s)
            .collect();
        let mut chain: Vec<RenderStep> = below.iter().map(|s| s.render_step()).collect();
        let mut h = below
            .last()
            .and_then(|s| s.stack_hash.clone())
            .unwrap_or_else(|| self.manifest.source.sha256.clone());
        let mut input_hash = match below.last() {
            Some(s) => s.output_hash.clone().unwrap_or_default(),
            None => self.source_render_hash().to_string(),
        };
        let keep = self.state.stack_hash.clone();
        let mut out = Vec::new();
        for (s, on) in &places[k..] {
            if !on {
                continue;
            }
            let mut s = s.clone();
            let old_hash = s.stack_hash.clone();
            chain.push(s.render_step());
            h = step::next_stack_hash(&h, &s.render_step());
            let measured_on = s.resolved_on.take().unwrap_or_else(|| s.input_hash.clone());
            let changed = fresh == Some(s.step_id.as_str());
            if !changed && s.input_hash == input_hash && s.output_hash.is_some() {
                // The same input and values: the same output.
                if let Some(old) = &old_hash {
                    self.renders.alias(old, &h, &keep);
                }
            } else {
                let (input, _) = self.render_prefix(&chain[..chain.len() - 1])?;
                let (output, meas) = self.render_prefix(&chain)?;
                s.input_hash = input_hash.clone();
                let before = self.snapshot(&input, &s.scope);
                s.measurements = meas;
                s.output_hash = Some(output.render_hash());
                s.state_after = Some(self.snapshot(&output, &s.scope));
                s.bindings
                    .retain(|_, b| b.source == BindingSource::Declared);
                s.bindings = bindings::infer(&s, &before.clip, &input);
                s.state_before = Some(before);
            }
            if timeline::is_edit(&s.op) {
                s.measurements.insert(
                    "output_duration_s".into(),
                    json!(self.output_duration_s(&chain)),
                );
            }
            let op = reg.get(&s.op, s.op_version)?;
            s.resolved_on = (op.measures_input(&s.params) && measured_on != s.input_hash)
                .then_some(measured_on);
            s.stack_hash = Some(h.clone());
            input_hash = s.output_hash.clone().unwrap_or_default();
            out.push(s);
        }
        Ok(out)
    }

    pub fn annotate(
        &mut self,
        env: &mut dyn Env,
        step_id: &str,
        note: Option<String>,
        labels: Vec<String>,
        actor: Option<Actor>,
    ) -> Result<()> {
        if !self.state.steps.iter().any(|s| s.step_id == step_id) {
            return invalid(format!("no step `{step_id}` on the stack"));
        }
        self.append(
            env,
            "step.annotated",
            &actor.unwrap_or_else(Actor::user),
            json!({ "step_id": step_id, "note": note, "labels": labels }),
        )?;
        Ok(())
    }

    pub fn rate(&mut self, env: &mut dyn Env, rating: Rating, actor: Option<Actor>) -> Result<()> {
        if !(1..=5).contains(&rating.overall) || rating.dims.values().any(|v| !(1..=5).contains(v))
        {
            return invalid("ratings are 1–5");
        }
        self.append(
            env,
            "stack.rated",
            &actor.unwrap_or_else(Actor::user),
            to_json(&rating),
        )?;
        Ok(())
    }

    /// The stack's output with the final limiter (always last).
    /// Time edits are applied to the processed result, then the limiter.
    pub fn render_final(&mut self) -> Result<FinalRender> {
        let steps = self.render_steps();
        let (audio, _) = self.render_prefix(&steps)?;
        let layout = self.edit_layout();
        let audio = if layout.is_identity() {
            audio
        } else {
            Arc::new(layout.apply(&audio))
        };
        let lim = engine::final_limiter(Some(&self.manifest.final_limiter))?;
        let out = engine::render_full(&audio, std::slice::from_ref(&lim))?;
        let sr = self.source.sample_rate;
        Ok(FinalRender {
            output_hash: out.audio.render_hash(),
            stack_hash: step::stack_hash(&self.state.stack_hash, std::slice::from_ref(&lim)),
            audio: out.audio,
            limiter: out.measurements.into_iter().next().unwrap_or_default(),
            edits: layout.marks_json(sr),
            cues: layout.cues(sr),
        })
    }

    /// Length of the output (seconds) that a chain of steps would give.
    fn output_duration_s(&self, chain: &[RenderStep]) -> f64 {
        let l = timeline::layout(&timeline::edits(chain), self.source.len());
        crate::math::round_to(l.output_len() as f64 / self.source.sample_rate as f64, 6)
    }

    /// How the output is assembled from the original under the stack's time edits.
    pub fn edit_layout(&self) -> timeline::Layout {
        timeline::layout(&timeline::edits(&self.render_steps()), self.source.len())
    }

    /// Record an outward action (an export, a saved recipe, …).
    pub fn record(
        &mut self,
        env: &mut dyn Env,
        kind: &str,
        actor: Option<Actor>,
        data: Value,
    ) -> Result<Value> {
        self.append(env, kind, &actor.unwrap_or_else(Actor::user), data)
    }

    /// Clone this project at a step (or its current state) into `dest`.
    pub fn clone_into<S2: Store>(
        &mut self,
        env: &mut dyn Env,
        mut dest: S2,
        at_step: Option<&str>,
        name: Option<&str>,
        actor: Option<Actor>,
    ) -> Result<Project<S2>> {
        let actor = actor.unwrap_or_else(Actor::user);
        let upto = match at_step {
            None => self.state.steps.len(),
            Some(id) => {
                self.state
                    .steps
                    .iter()
                    .position(|s| s.step_id == id)
                    .ok_or_else(|| ProjectError::Invalid(format!("no step `{id}` on the stack")))?
                    + 1
            }
        };
        let fork_step = if upto > 0 {
            Some(self.state.steps[upto - 1].step_id.clone())
        } else {
            None
        };
        let fork_event = if upto > 0 {
            Some(self.state.step_events[upto - 1].clone())
        } else {
            None
        };
        let child_id = env.new_id("pr");
        let child_name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} (clone)", self.manifest.project.name));
        let made = self.append(
            env,
            "project.clone_made",
            &actor,
            json!({ "child_project": child_id, "child_name": child_name, "at_step": fork_step, "steps_inherited": upto }),
        )?;
        let head = made["hash"].as_str().expect("hash").to_string();

        // Source (byte-for-byte), and the ancestry needed to verify back to import.
        let bytes = self.source_bytes()?;
        dest.write_new(&self.manifest.source.path, &bytes)?;
        dest.write_new(
            &format!("lineage/{}.jsonl", self.id()),
            self.log.to_jsonl().as_bytes(),
        )?;
        for f in self
            .store
            .list()
            .into_iter()
            .filter(|f| f.starts_with("lineage/"))
        {
            dest.write_new(&f, &self.store.read(&f)?)?;
        }
        let mut ancestors = vec![self.id().to_string()];
        if let Some(l) = &self.manifest.lineage {
            ancestors.extend(l.ancestors.iter().cloned());
        }
        let inherited: Vec<Step> = self.state.steps[..upto]
            .iter()
            .zip(&self.state.step_events)
            .map(|(s, ev)| {
                let mut s = s.clone();
                s.inherited_from = Some(step::InheritedRef {
                    project: self.id().to_string(),
                    step_id: s.step_id.clone(),
                    event_hash: ev.clone(),
                });
                s
            })
            .collect();
        let created = env.now();
        let stack_now = step::stack_hash(
            &self.manifest.source.sha256,
            &inherited.iter().map(Step::render_step).collect::<Vec<_>>(),
        );
        let manifest = Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            project: ProjectInfo {
                id: child_id.clone(),
                name: child_name.clone(),
                created,
            },
            source: self.manifest.source.clone(),
            source_info: self.manifest.source_info.clone(),
            lineage: Some(Lineage {
                parent_project: self.id().to_string(),
                parent_name: self.manifest.project.name.clone(),
                parent_head: head.clone(),
                forked_at_step: fork_step.clone(),
                forked_at_event: fork_event.clone(),
                ancestors,
            }),
            view: self.manifest.view.clone(),
            log: LogHead {
                events: 0,
                head: head.clone(),
            },
            stack: StackSummary {
                steps: vec![],
                stack_hash: self.manifest.source.sha256.clone(),
            },
            final_limiter: self.manifest.final_limiter.clone(),
        };
        let mut child = Project {
            store: dest,
            manifest,
            log: EventLog::new(&head),
            app: self.app.clone(),
            source: self.source.clone(),
            state: StackState {
                stack_hash: self.manifest.source.sha256.clone(),
                ..Default::default()
            },
            renders: cache::RenderCache::default(),
            features_cache: self.features_cache.clone(),
            cache_renders: false,
            read_only: None,
            source_render_hash: std::cell::OnceCell::new(),
        };
        child.append(
            env,
            "project.cloned_from",
            &actor,
            json!({
                "project_id": child_id,
                "name": child_name,
                "parent_project": self.id(),
                "parent_name": self.manifest.project.name,
                "parent_head": { "events": self.log.len(), "hash": head },
                "forked_at_step": fork_step,
                "forked_at_event": fork_event,
                "source": self.manifest.source,
                "inherited_steps": inherited,
            }),
        )?;
        debug_assert_eq!(child.state.stack_hash, stack_now);
        Ok(child)
    }

    /// Pack the project as a portable bundle (derived renders excluded). The
    /// export itself is logged first, so the bundle carries its own record.
    pub fn export_bundle(&mut self, env: &mut dyn Env, actor: Option<Actor>) -> Result<Vec<u8>> {
        let files: Vec<String> = self
            .store
            .list()
            .into_iter()
            .filter(|f| !f.starts_with("renders/"))
            .collect();
        self.append(
            env,
            "bundle.exported",
            &actor.unwrap_or_else(Actor::user),
            json!({ "files": files.len() }),
        )?;
        let mut mem = MemStore::new();
        for f in self
            .store
            .list()
            .into_iter()
            .filter(|f| !f.starts_with("renders/"))
        {
            mem.files.insert(f.clone(), self.store.read(&f)?);
        }
        Ok(bundle::pack(&mem)?)
    }
}

/// Open a `.nlae` bundle into memory.
pub fn open_bundle(bytes: &[u8], app: AppInfo) -> Result<(Project<MemStore>, OpenReport)> {
    let store = bundle::unpack(bytes)?;
    Project::open(store, app)
}

/// Verify an ancestor's log (and, recursively, its ancestors) from `lineage/`.
fn verify_ancestor<S: Store>(
    store: &S,
    project: &str,
    expected_head: &str,
    source_sha: &str,
    out: &mut Vec<LineageCheck>,
    problems: &mut Vec<String>,
    depth: usize,
) {
    if depth > 64 {
        problems.push("lineage is too deep".into());
        return;
    }
    let path = format!("lineage/{project}.jsonl");
    let text = match store
        .read(&path)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
    {
        Some(t) => t,
        None => {
            problems.push(format!("missing ancestor log `{path}`"));
            return;
        }
    };
    let (log, v) = EventLog::parse(&text, None);
    if let Some(f) = &v.first_failure {
        problems.push(format!(
            "ancestor `{project}` log broken at line {} (seq {}): {}",
            f.line, f.expected_seq, f.reason
        ));
    } else if log.head_hash() != expected_head {
        problems.push(format!(
            "ancestor `{project}` log does not end at the fork event {expected_head}"
        ));
    }
    let first = log.events().first().cloned().unwrap_or(Value::Null);
    out.push(LineageCheck {
        project: project.to_string(),
        verification: v,
    });
    match first["type"].as_str() {
        Some("project.cloned_from") => {
            let parent = first["data"]["parent_project"].as_str().unwrap_or("");
            let head = first["data"]["parent_head"]["hash"].as_str().unwrap_or("");
            if first["prev"].as_str() != Some(head) {
                problems.push(format!(
                    "ancestor `{project}` does not start from its parent's fork event"
                ));
            }
            verify_ancestor(store, parent, head, source_sha, out, problems, depth + 1);
        }
        Some("project.created") => {
            if log.genesis_prev() != ZERO_HASH {
                problems.push(format!(
                    "original ancestor `{project}` does not start from the zero hash"
                ));
            }
            let imported = log.events().iter().find(|e| {
                matches!(
                    e["type"].as_str(),
                    Some("source.imported") | Some("source.recorded")
                )
            });
            match imported.and_then(|e| e["data"]["source"]["sha256"].as_str()) {
                Some(h) if h == source_sha => {}
                Some(h) => problems.push(format!(
                    "the source differs from the one imported by `{project}` ({h})"
                )),
                None => problems.push(format!("ancestor `{project}` records no import")),
            }
        }
        _ => problems.push(format!(
            "ancestor `{project}` log has an unexpected first event"
        )),
    }
}

#[cfg(test)]
mod tests;
