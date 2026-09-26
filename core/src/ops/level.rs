//! Gain, DC removal, and normalisation to a peak or a loudness.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{copy_window, typed, with, Descriptor, Op, OpError, RenderOut};
use crate::analysis::loudness::{gain_to_target, integrated_lufs};
use crate::audio::AudioBuffer;
use crate::dsp::window::ramp;
use crate::math::{amp_to_db, db_to_amp, round_to};
use crate::scope::Scope;

/// Fade length at the edges of a time-range scope.
pub const SCOPE_RAMP_S: f64 = 0.005;

/// Weight (0–1) of a scoped constant change at absolute sample `i`: 1 inside
/// the scope, fading over `ramp_len` samples at interior edges, 0 outside.
pub(crate) fn scope_weight(i: i64, s0: i64, s1: i64, clip_len: i64, ramp_len: i64) -> f64 {
    let whole_left = s0 <= 0;
    let whole_right = s1 >= clip_len;
    if (i < s0 && !whole_left) || (i >= s1 && !whole_right) {
        return 0.0;
    }
    let mut w = 1.0;
    if !whole_left && i - s0 < ramp_len {
        w = ramp((i - s0) as usize, ramp_len as usize);
    }
    if !whole_right && s1 - 1 - i < ramp_len {
        w = w.min(ramp((s1 - 1 - i) as usize, ramp_len as usize));
    }
    w
}

fn peak_over(buf: &AudioBuffer) -> f64 {
    buf.channels
        .iter()
        .flat_map(|c| c.iter())
        .fold(0.0f64, |m, &x| m.max((x as f64).abs()))
}

/// Apply a scoped gain (in dB) to a window. Samples with weight 0 or a
/// 0 dB gain are copied bit-exactly.
fn apply_scoped_gain(
    input: &AudioBuffer,
    offset: i64,
    a: i64,
    b: i64,
    scope: &Scope,
    clip_len: usize,
    gain_db: f64,
) -> AudioBuffer {
    let mut out = copy_window(input, offset, a, b);
    if gain_db == 0.0 {
        return out;
    }
    let (s0, s1) = scope.samples(input.sample_rate, clip_len);
    let ramp_len = (SCOPE_RAMP_S * input.sample_rate as f64).round() as i64;
    let g = db_to_amp(gain_db);
    for ch in out.channels.iter_mut() {
        for (j, x) in ch.iter_mut().enumerate() {
            let i = a + j as i64;
            let w = scope_weight(i, s0 as i64, s1 as i64, clip_len as i64, ramp_len);
            if w == 0.0 {
                continue;
            }
            let factor = if w == 1.0 { g } else { 1.0 + (g - 1.0) * w };
            *x = (*x as f64 * factor) as f32;
        }
    }
    out
}

fn level_measurements(before: &AudioBuffer, after: &AudioBuffer) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(
        "peak_before_dbfs".into(),
        json!(round_to(amp_to_db(peak_over(before)), 2)),
    );
    m.insert(
        "peak_after_dbfs".into(),
        json!(round_to(amp_to_db(peak_over(after)), 2)),
    );
    m
}

// ---------------------------------------------------------------- gain

pub struct Gain {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GainParams {
    gain_db: f64,
}

impl Op for Gain {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        _scope: &Scope,
        _input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        typed::<GainParams>(params)?;
        Ok(params.clone())
    }

    fn radius(&self, _resolved: &Value, _sr: u32) -> usize {
        0
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let p: GainParams = typed(resolved)?;
        let len = target.len();
        Ok(Some(apply_scoped_gain(
            target, 0, 0, len as i64, scope, len, p.gain_db,
        )))
    }

    fn render(
        &self,
        resolved: &Value,
        scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let p: GainParams = typed(resolved)?;
        let before = copy_window(input, offset, a, b);
        let audio = apply_scoped_gain(input, offset, a, b, scope, clip_len, p.gain_db);
        let measurements = level_measurements(&before, &audio);
        Ok(RenderOut {
            audio,
            measurements,
        })
    }
}

// ---------------------------------------------------------------- normalise

pub struct Normalise {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NormaliseParams {
    target_peak_dbfs: f64,
}

#[derive(Deserialize)]
struct NormaliseResolved {
    gain_db: f64,
}

impl Op for Normalise {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn measures_input(&self, _params: &Value) -> bool {
        true
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: NormaliseParams = typed(params)?;
        let (s0, s1) = scope.samples(input.sample_rate, input.len());
        let peak = input
            .channels
            .iter()
            .flat_map(|c| c[s0..s1].iter())
            .fold(0.0f64, |m, &x| m.max((x as f64).abs()));
        if peak <= 0.0 {
            return Err(OpError::Invalid(
                "cannot normalise silence: the scope has no signal".into(),
            ));
        }
        let measured = amp_to_db(peak);
        Ok(with(
            params,
            json!({ "measured_peak_dbfs": measured, "gain_db": p.target_peak_dbfs - measured }),
        ))
    }

    fn radius(&self, _resolved: &Value, _sr: u32) -> usize {
        0
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: NormaliseResolved = typed(resolved)?;
        let len = target.len();
        Ok(Some(apply_scoped_gain(
            target, 0, 0, len as i64, scope, len, r.gain_db,
        )))
    }

    fn render(
        &self,
        resolved: &Value,
        scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let r: NormaliseResolved = typed(resolved)?;
        let before = copy_window(input, offset, a, b);
        let audio = apply_scoped_gain(input, offset, a, b, scope, clip_len, r.gain_db);
        let mut measurements = level_measurements(&before, &audio);
        measurements.insert("gain_db".into(), json!(round_to(r.gain_db, 3)));
        Ok(RenderOut {
            audio,
            measurements,
        })
    }
}

// ---------------------------------------------------------------- loudness_normalise

pub struct LoudnessNormalise {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoudnessParams {
    target_lufs: f64,
}

#[derive(Deserialize)]
struct LoudnessResolved {
    measured_lufs: f64,
    gain_db: f64,
}

impl Op for LoudnessNormalise {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn measures_input(&self, _params: &Value) -> bool {
        true
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: LoudnessParams = typed(params)?;
        let (s0, s1) = scope.samples(input.sample_rate, input.len());
        let measured = integrated_lufs(input, s0, s1).ok_or_else(|| {
            OpError::Invalid(
                "cannot measure the loudness: the scope is shorter than 0.4 s, or nothing in it is louder than −70 LUFS".into(),
            )
        })?;
        Ok(with(
            params,
            json!({ "measured_lufs": measured, "gain_db": gain_to_target(measured, p.target_lufs) }),
        ))
    }

    fn radius(&self, _resolved: &Value, _sr: u32) -> usize {
        0
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: LoudnessResolved = typed(resolved)?;
        let len = target.len();
        Ok(Some(apply_scoped_gain(
            target, 0, 0, len as i64, scope, len, r.gain_db,
        )))
    }

    fn render(
        &self,
        resolved: &Value,
        scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let r: LoudnessResolved = typed(resolved)?;
        let before = copy_window(input, offset, a, b);
        let audio = apply_scoped_gain(input, offset, a, b, scope, clip_len, r.gain_db);
        let mut measurements = level_measurements(&before, &audio);
        measurements.insert("gain_db".into(), json!(round_to(r.gain_db, 3)));
        // Measured on the whole scope when resolving; a fixed gain moves it exactly.
        measurements.insert(
            "loudness_before_lufs".into(),
            json!(round_to(r.measured_lufs, 2)),
        );
        measurements.insert(
            "loudness_after_lufs".into(),
            json!(round_to(r.measured_lufs + r.gain_db, 2)),
        );
        Ok(RenderOut {
            audio,
            measurements,
        })
    }
}

// ---------------------------------------------------------------- dc_remove

pub struct DcRemove {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // parsed to check the parameter set against the descriptor
struct DcParams {
    mode: String,
    window_ms: f64,
}

#[derive(Deserialize)]
struct DcResolved {
    mode: String,
    window_ms: f64,
    #[serde(default)]
    offsets: Vec<f64>,
}

/// Fixed-point scale for exact moving sums: integer addition is associative, so
/// a window's sum is the same whichever sample a render starts from.
const FIXED: f64 = 4_294_967_296.0; // 2^32

fn drift_half(window_ms: f64, sr: u32) -> i64 {
    ((window_ms / 1000.0 * sr as f64) / 2.0).round().max(1.0) as i64
}

impl Op for DcRemove {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn measures_input(&self, params: &Value) -> bool {
        params["mode"] == "mean"
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: DcParams = typed(params)?;
        if p.mode == "mean" {
            let (s0, s1) = scope.samples(input.sample_rate, input.len());
            let n = (s1 - s0).max(1) as f64;
            let offsets: Vec<f64> = input
                .channels
                .iter()
                .map(|c| c[s0..s1].iter().map(|&x| x as f64).sum::<f64>() / n)
                .collect();
            Ok(with(params, json!({ "offsets": offsets })))
        } else {
            Ok(params.clone())
        }
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        match typed::<DcResolved>(resolved) {
            Ok(r) if r.mode == "drift" => drift_half(r.window_ms, sr) as usize,
            _ => 0,
        }
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        // Subtracting a mean (or a moving average) is linear: each component
        // loses its own share, so apply it to the target's own values.
        let mut params = resolved.clone();
        if let Some(m) = params.as_object_mut() {
            m.remove("offsets");
        }
        let own = self.resolve(&params, scope, target)?;
        let len = target.len();
        Ok(Some(
            self.render(&own, scope, target, 0, 0, len as i64, len)?
                .audio,
        ))
    }

    fn render(
        &self,
        resolved: &Value,
        scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let r: DcResolved = typed(resolved)?;
        let sr = input.sample_rate;
        let (s0, s1) = scope.samples(sr, clip_len);
        let ramp_len = (SCOPE_RAMP_S * sr as f64).round() as i64;
        let mut out = copy_window(input, offset, a, b);
        let mut removed = Vec::new();
        for (ci, ch) in out.channels.iter_mut().enumerate() {
            let src = &input.channels[ci];
            // Offset to subtract at absolute sample i.
            let offsets: Vec<f64> = if r.mode == "mean" {
                vec![r.offsets.get(ci).copied().unwrap_or(0.0); ch.len()]
            } else {
                let half = drift_half(r.window_ms, sr);
                let width = (2 * half + 1) as f64;
                let q = |i: i64| -> i128 {
                    let j = i - offset;
                    if j >= 0 && (j as usize) < src.len() && i >= 0 && (i as usize) < clip_len {
                        (src[j as usize] as f64 * FIXED).round() as i128
                    } else {
                        0
                    }
                };
                let mut sum: i128 = (a - half..=a + half).map(q).sum();
                let mut v = Vec::with_capacity(ch.len());
                for j in 0..ch.len() as i64 {
                    let i = a + j;
                    if j > 0 {
                        sum += q(i + half) - q(i - half - 1);
                    }
                    v.push(sum as f64 / FIXED / width);
                }
                v
            };
            let mut total = 0.0;
            for (j, x) in ch.iter_mut().enumerate() {
                let i = a + j as i64;
                let w = scope_weight(i, s0 as i64, s1 as i64, clip_len as i64, ramp_len);
                let off = offsets[j] * w;
                total += off;
                if off != 0.0 {
                    *x = (*x as f64 - off) as f32;
                }
            }
            removed.push(round_to(total / ch.len().max(1) as f64, 9));
        }
        let mut measurements = Map::new();
        measurements.insert("offset_removed".into(), json!(removed));
        Ok(RenderOut {
            audio: out,
            measurements,
        })
    }
}
