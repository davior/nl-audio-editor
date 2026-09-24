//! Rendering a stack of resolved steps over the source.
//!
//! - [`render_full`] runs each step over the whole clip in turn.
//! - [`render_window`] renders only `[a, b)`: each step asks the step before it
//!   for its window widened by its own radius, recursively. Because every
//!   operation's reach is finite and its frame grid is anchored to the clip
//!   start, the result is bit-identical to the same span of a full render.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::audio::AudioBuffer;
use crate::ops::{registry, Op, OpError};
use crate::scope::Scope;

/// A step reduced to what rendering needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderStep {
    pub op: String,
    pub op_version: u32,
    pub resolved: Value,
    pub scope: Scope,
}

impl RenderStep {
    pub fn op(&self) -> Result<&'static dyn Op, OpError> {
        registry().get(&self.op, self.op_version)
    }
}

/// Output of a whole-clip render, with each step's measurements.
pub struct FullRender {
    pub audio: AudioBuffer,
    pub measurements: Vec<Map<String, Value>>,
}

pub fn render_full(source: &AudioBuffer, steps: &[RenderStep]) -> Result<FullRender, OpError> {
    let len = source.len();
    let mut cur = source.clone();
    let mut measurements = Vec::with_capacity(steps.len());
    for s in steps {
        let out = s
            .op()?
            .render(&s.resolved, &s.scope, &cur, 0, 0, len as i64, len)?;
        measurements.push(out.measurements);
        cur = out.audio;
    }
    Ok(FullRender {
        audio: cur,
        measurements,
    })
}

/// Render absolute samples `[a, b)` of the stack's output.
pub fn render_window(
    source: &AudioBuffer,
    steps: &[RenderStep],
    a: i64,
    b: i64,
) -> Result<AudioBuffer, OpError> {
    Ok(render_window_measured(source, steps, a, b)?.0)
}

/// As [`render_window`], also returning the last step's window measurements.
pub fn render_window_measured(
    source: &AudioBuffer,
    steps: &[RenderStep],
    a: i64,
    b: i64,
) -> Result<(AudioBuffer, Map<String, Value>), OpError> {
    let len = source.len() as i64;
    let Some((last, before)) = steps.split_last() else {
        return Ok((source.extract_padded(a, b), Map::new()));
    };
    let op = last.op()?;
    let r = op.radius(&last.resolved, source.sample_rate) as i64;
    let (a2, b2) = ((a - r).max(0), (b + r).min(len));
    let input = if a2 < b2 {
        render_window(source, before, a2, b2)?
    } else {
        AudioBuffer::silent(source.sample_rate, source.num_channels(), 0)
    };
    let out = op.render(&last.resolved, &last.scope, &input, a2, a, b, source.len())?;
    Ok((out.audio, out.measurements))
}

/// `a − b` sample by sample (the residual: what the processing removed).
pub fn difference(a: &AudioBuffer, b: &AudioBuffer) -> AudioBuffer {
    let channels = a
        .channels
        .iter()
        .zip(&b.channels)
        .map(|(x, y)| x.iter().zip(y).map(|(p, q)| p - q).collect())
        .collect();
    AudioBuffer::new(a.sample_rate, channels)
}

/// The system limiter every render ends with.
pub fn final_limiter(params: Option<&Value>) -> Result<RenderStep, OpError> {
    let reg = registry();
    let op = reg.latest("limiter")?;
    let desc = op.descriptor();
    let p = desc.normalise_params(params.unwrap_or(&Value::Null))?;
    Ok(RenderStep {
        op: desc.id.clone(),
        op_version: desc.version,
        resolved: p,
        scope: Scope::Clip,
    })
}

/// Render each named component through `steps`, with every decision (mask,
/// envelope, offset) taken from the mixture's own render at that point. The
/// sum of the results equals the rendered mixture up to float rounding, and
/// each result shows exactly what the stack did to that component.
pub fn render_components(
    mix: &AudioBuffer,
    steps: &[RenderStep],
    components: &std::collections::BTreeMap<String, AudioBuffer>,
) -> Result<(AudioBuffer, std::collections::BTreeMap<String, AudioBuffer>), OpError> {
    let len = mix.len();
    let mut cur_mix = mix.clone();
    let mut cur = components.clone();
    for s in steps {
        let op = s.op()?;
        for (name, c) in cur.iter_mut() {
            *c = op
                .render_linear(&s.resolved, &s.scope, &cur_mix, c)?
                .ok_or_else(|| {
                    OpError::Invalid(format!("{} cannot render component `{name}`", s.op))
                })?;
        }
        cur_mix = op
            .render(&s.resolved, &s.scope, &cur_mix, 0, 0, len as i64, len)?
            .audio;
    }
    Ok((cur_mix, cur))
}
