//! The operation registry and the operation contract.
//!
//! An operation is used in two phases:
//! 1. **resolve** — on its actual input, turn the validated parameters (which
//!    may say `auto`) into concrete values. The result, `resolved`, is the
//!    step's exact form and the only thing rendering reads.
//! 2. **render** — produce absolute output samples `[a, b)` from an input
//!    buffer whose first sample is absolute sample `offset`. An operation
//!    declares its `radius`: output sample `s` depends only on input samples
//!    within `s ± radius`. That finite reach is what makes a preview of a window
//!    bit-identical to the same span of a full render.

pub mod compressor;
pub mod descriptor;
pub mod edit;
pub mod eq;
pub mod gate;
pub mod hum;
pub mod level;
pub mod limiter;
pub mod mask;
pub mod noise_reduce;
pub mod spectral;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::audio::AudioBuffer;
use crate::scope::{Scope, ScopeError};
pub use descriptor::{Descriptor, OpClass, ParamError};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("unknown operation `{0}`")]
    Unknown(String),
    #[error(transparent)]
    Param(#[from] ParamError),
    #[error("scope `{kind}` is not supported by `{op}` (supported: {supported})")]
    ScopeKind {
        op: String,
        kind: String,
        supported: String,
    },
    #[error(transparent)]
    Scope(#[from] ScopeError),
    #[error("{0}")]
    Invalid(String),
}

/// Output of a render: audio for `[a, b)` plus what the DSP measured doing it.
pub struct RenderOut {
    pub audio: AudioBuffer,
    pub measurements: Map<String, Value>,
}

pub trait Op: Send + Sync {
    fn descriptor(&self) -> &Descriptor;

    /// Concrete values for this clip. `params` are already validated and complete.
    fn resolve(&self, params: &Value, scope: &Scope, input: &AudioBuffer)
        -> Result<Value, OpError>;

    /// Input samples needed on each side of any output sample.
    fn radius(&self, resolved: &Value, sample_rate: u32) -> usize;

    /// Render absolute output samples `[a, b)`. `input` starts at absolute
    /// sample `offset`; samples outside it are treated as silence, which is
    /// correct beyond the clip and never reached inside it when the caller
    /// honours `radius`.
    fn render(
        &self,
        resolved: &Value,
        scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        clip_len: usize,
    ) -> Result<RenderOut, OpError>;

    /// Render the whole clip of `target`, with every signal-dependent decision
    /// (mask, envelope, offset) taken from `decide` instead. Once its decisions
    /// are fixed, each operation is linear, so this measures exactly what a step
    /// did to one known component of a mixture (the golden clips rely on it).
    /// `None` if the operation cannot do this.
    fn render_linear(
        &self,
        _resolved: &Value,
        _scope: &Scope,
        _decide: &AudioBuffer,
        _target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        Ok(None)
    }
}

/// Parse validated parameters or resolved values into an op's typed struct.
pub(crate) fn typed<T: DeserializeOwned>(v: &Value) -> Result<T, OpError> {
    serde_json::from_value(v.clone())
        .map_err(|e| OpError::Invalid(format!("internal parameter mismatch: {e}")))
}

/// Merge extra resolved fields into the normalised parameters.
pub(crate) fn with(params: &Value, extra: Value) -> Value {
    let mut m = params.as_object().cloned().unwrap_or_default();
    if let Value::Object(e) = extra {
        m.extend(e);
    }
    Value::Object(m)
}

/// Copy `[a, b)` of `input` (absolute positions) into a new buffer.
pub(crate) fn copy_window(input: &AudioBuffer, offset: i64, a: i64, b: i64) -> AudioBuffer {
    input.extract_padded(a - offset, b - offset)
}

pub struct Registry {
    ops: BTreeMap<(String, u32), Box<dyn Op>>,
}

const DESCRIPTORS: &[&str] = &[
    include_str!("../../../schemas/ops/gain.v1.json"),
    include_str!("../../../schemas/ops/dc_remove.v1.json"),
    include_str!("../../../schemas/ops/normalise.v1.json"),
    include_str!("../../../schemas/ops/compressor.v1.json"),
    include_str!("../../../schemas/ops/limiter.v1.json"),
    include_str!("../../../schemas/ops/noise_reduce.v1.json"),
    include_str!("../../../schemas/ops/line_reduce.v1.json"),
    include_str!("../../../schemas/ops/line_reduce.v2.json"),
    include_str!("../../../schemas/ops/band_cut.v1.json"),
    include_str!("../../../schemas/ops/spectral_compressor.v1.json"),
    include_str!("../../../schemas/ops/remove_time.v1.json"),
    include_str!("../../../schemas/ops/insert_silence.v1.json"),
    include_str!("../../../schemas/ops/high_pass.v1.json"),
    include_str!("../../../schemas/ops/low_pass.v1.json"),
    include_str!("../../../schemas/ops/bell.v1.json"),
    include_str!("../../../schemas/ops/shelf.v1.json"),
    include_str!("../../../schemas/ops/tilt.v1.json"),
    include_str!("../../../schemas/ops/gate.v1.json"),
    include_str!("../../../schemas/ops/loudness_normalise.v1.json"),
    include_str!("../../../schemas/ops/hum_reduce.v1.json"),
];

fn build(desc: Descriptor) -> Box<dyn Op> {
    match (desc.id.as_str(), desc.version) {
        ("gain", 1) => Box::new(level::Gain { desc }),
        ("dc_remove", 1) => Box::new(level::DcRemove { desc }),
        ("normalise", 1) => Box::new(level::Normalise { desc }),
        ("compressor", 1) => Box::new(compressor::Compressor { desc }),
        ("limiter", 1) => Box::new(limiter::Limiter { desc }),
        ("noise_reduce", 1) => Box::new(noise_reduce::NoiseReduce { desc }),
        ("line_reduce", 1 | 2) => Box::new(spectral::LineReduce { desc }),
        ("band_cut", 1) => Box::new(spectral::BandCut { desc }),
        ("spectral_compressor", 1) => Box::new(spectral::SpectralCompressor { desc }),
        ("remove_time", 1) => Box::new(edit::RemoveTime { desc }),
        ("insert_silence", 1) => Box::new(edit::InsertSilence { desc }),
        ("high_pass", 1) => Box::new(eq::Pass { desc, high: true }),
        ("low_pass", 1) => Box::new(eq::Pass { desc, high: false }),
        ("bell", 1) => Box::new(eq::Bell { desc }),
        ("shelf", 1) => Box::new(eq::Shelf { desc }),
        ("tilt", 1) => Box::new(eq::Tilt { desc }),
        ("gate", 1) => Box::new(gate::Gate { desc }),
        ("loudness_normalise", 1) => Box::new(level::LoudnessNormalise { desc }),
        ("hum_reduce", 1) => Box::new(hum::HumReduce { desc }),
        (id, v) => panic!("descriptor {id} v{v} has no implementation"),
    }
}

impl Registry {
    fn load() -> Self {
        let mut ops = BTreeMap::new();
        for text in DESCRIPTORS {
            let desc: Descriptor =
                serde_json::from_str(text).expect("embedded descriptors are valid");
            ops.insert((desc.id.clone(), desc.version), build(desc));
        }
        Registry { ops }
    }

    pub fn get(&self, id: &str, version: u32) -> Result<&dyn Op, OpError> {
        self.ops
            .get(&(id.to_string(), version))
            .map(|b| b.as_ref())
            .ok_or_else(|| OpError::Unknown(format!("{id} v{version}")))
    }

    /// The latest version of an operation.
    pub fn latest(&self, id: &str) -> Result<&dyn Op, OpError> {
        self.ops
            .range((id.to_string(), 0)..=(id.to_string(), u32::MAX))
            .next_back()
            .map(|(_, b)| b.as_ref())
            .ok_or_else(|| OpError::Unknown(id.to_string()))
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &Descriptor> {
        self.ops.values().map(|o| o.descriptor())
    }

    /// Validate params and scope against the descriptor. Returns complete params.
    pub fn validate(
        &self,
        id: &str,
        version: u32,
        params: &Value,
        scope: &Scope,
    ) -> Result<Value, OpError> {
        let op = self.get(id, version)?;
        let desc = op.descriptor();
        if !desc.scopes.contains(&scope.kind()) {
            let name = |k| {
                serde_json::to_value(k)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            return Err(OpError::ScopeKind {
                op: id.to_string(),
                kind: name(scope.kind()),
                supported: desc
                    .scopes
                    .iter()
                    .map(|k| name(*k))
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        Ok(desc.normalise_params(params)?)
    }
}

pub fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(Registry::load)
}

#[cfg(test)]
mod tests;
