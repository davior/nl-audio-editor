//! Time edits as operations: validated, resolved and recorded like any other,
//! but they leave the processing as it is. The removal or the silence is
//! applied to the processed result, just before the final limiter (see
//! [`crate::timeline`]); resolving writes down exactly which samples.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{copy_window, typed, with, Descriptor, Op, OpError, RenderOut};
use crate::audio::AudioBuffer;
use crate::math::round_to;
use crate::scope::Scope;

fn samples(t: f64, sample_rate: u32) -> usize {
    (t * sample_rate as f64).round().max(0.0) as usize
}

fn pass_through(
    input: &AudioBuffer,
    offset: i64,
    a: i64,
    b: i64,
    measurements: Map<String, Value>,
) -> RenderOut {
    RenderOut {
        audio: copy_window(input, offset, a, b),
        measurements,
    }
}

pub struct RemoveTime {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
struct RemoveParams {
    fade_ms: f64,
}

impl Op for RemoveTime {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: RemoveParams = typed(params)?;
        let (s0, s1) = scope.samples(input.sample_rate, input.len());
        if s1 <= s0 {
            return Err(OpError::Invalid("the stretch to remove is empty".into()));
        }
        Ok(with(
            params,
            json!({
                "s0": s0,
                "s1": s1,
                "fade_samples": samples(p.fade_ms / 1000.0, input.sample_rate),
                "removed_s": round_to((s1 - s0) as f64 / input.sample_rate as f64, 6),
            }),
        ))
    }

    fn radius(&self, _resolved: &Value, _sample_rate: u32) -> usize {
        0
    }

    fn render(
        &self,
        resolved: &Value,
        _scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        _clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let mut m = Map::new();
        m.insert("removed_s".into(), resolved["removed_s"].clone());
        Ok(pass_through(input, offset, a, b, m))
    }
}

pub struct InsertSilence {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
struct InsertParams {
    at_s: f64,
    duration_s: f64,
    fade_ms: f64,
}

impl Op for InsertSilence {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        _scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: InsertParams = typed(params)?;
        let sr = input.sample_rate;
        let at = samples(p.at_s, sr);
        if at > input.len() {
            return Err(OpError::Invalid(format!(
                "at_s: {} s is after the end of the recording ({} s)",
                p.at_s,
                round_to(input.duration_s(), 3)
            )));
        }
        let len = samples(p.duration_s, sr).max(1);
        Ok(with(
            params,
            json!({
                "at_sample": at,
                "samples": len,
                "fade_samples": samples(p.fade_ms / 1000.0, sr),
                "inserted_s": round_to(len as f64 / sr as f64, 6),
            }),
        ))
    }

    fn radius(&self, _resolved: &Value, _sample_rate: u32) -> usize {
        0
    }

    fn render(
        &self,
        resolved: &Value,
        _scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        _clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let mut m = Map::new();
        m.insert("inserted_s".into(), resolved["inserted_s"].clone());
        Ok(pass_through(input, offset, a, b, m))
    }
}
