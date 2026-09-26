//! Broadband compressor for speech, with finite-support smoothing.
//!
//! The level is measured on a control grid (every ~0.67 ms, anchored to the
//! clip start) as the RMS of a 10 ms window, each window summed from scratch
//! so the result never depends on where a render starts. Reductions spread
//! with a look-ahead attack and a release of finite length, and the gain is
//! interpolated linearly between control points.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::level::{scope_weight, SCOPE_RAMP_S};
use super::{copy_window, typed, with, Descriptor, Op, OpError, RenderOut};
use crate::audio::AudioBuffer;
use crate::dsp::smooth::attack_release;
use crate::dsp::stft::div_floor;
use crate::math::{amp_to_db, db_to_amp, power_to_db, round_to};
use crate::scope::Scope;

pub struct Compressor {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // parsed to check the parameter set against the descriptor
struct Params {
    threshold_dbfs: f64,
    ratio: f64,
    knee_db: f64,
    attack_ms: f64,
    release_ms: f64,
    makeup_db: Value,
}

#[derive(Deserialize)]
struct Resolved {
    threshold_dbfs: f64,
    ratio: f64,
    knee_db: f64,
    attack_ms: f64,
    release_ms: f64,
    makeup_db: f64,
}

struct Grid {
    hop: i64,
    win: i64,
    att: usize,
    rel: usize,
}

/// The control grid, shared with the gate: a level every ~0.67 ms (anchored
/// to the clip start) from a 10 ms window. `(hop, window)` in samples.
pub(crate) fn control_grid(sr: u32) -> (i64, i64) {
    (
        ((sr as f64 / 1500.0).round() as i64).max(1),
        ((0.010 * sr as f64).round() as i64).max(1),
    )
}

/// RMS level (dB) of the `win` samples centred on `centre`, all channels
/// together, each window summed from scratch. Samples outside the clip or the
/// provided input read as zero.
pub(crate) fn window_level_db(
    input: &AudioBuffer,
    offset: i64,
    clip_len: usize,
    centre: i64,
    win: i64,
) -> f64 {
    let nch = input.num_channels() as f64;
    let sample = |ch: usize, i: i64| -> f64 {
        let j = i - offset;
        if i >= 0 && (i as usize) < clip_len && j >= 0 && (j as usize) < input.len() {
            input.channels[ch][j as usize] as f64
        } else {
            0.0
        }
    };
    let mut acc = 0.0;
    for ch in 0..input.num_channels() {
        for i in centre - win / 2..centre - win / 2 + win {
            let x = sample(ch, i);
            acc += x * x;
        }
    }
    power_to_db(acc / (win as f64 * nch))
}

fn grid(r: &Resolved, sr: u32) -> Grid {
    let (hop, win) = control_grid(sr);
    let frames = |ms: f64| ((ms / 1000.0 * sr as f64) / hop as f64).round() as usize;
    Grid {
        hop,
        win,
        att: frames(r.attack_ms),
        rel: frames(r.release_ms),
    }
}

/// Soft-knee static curve: reduction (dB, ≤ 0) for `over` dB above threshold.
pub(crate) fn knee_curve(over: f64, ratio: f64, knee: f64) -> f64 {
    let slope = 1.0 - 1.0 / ratio;
    if slope == 0.0 {
        return 0.0;
    }
    if knee > 0.0 && over > -knee / 2.0 && over < knee / 2.0 {
        -slope * (over + knee / 2.0) * (over + knee / 2.0) / (2.0 * knee)
    } else if over >= knee / 2.0 {
        -slope * over
    } else {
        0.0
    }
}

/// Envelope decided from `input`, applied to `target` (normally the same buffer).
fn render_with(
    r: &Resolved,
    scope: &Scope,
    input: &AudioBuffer,
    target: &AudioBuffer,
    offset: i64,
    a: i64,
    b: i64,
    clip_len: usize,
) -> (AudioBuffer, Vec<f32>) {
    let sr = input.sample_rate;
    let g = grid(r, sr);
    let ctx = (g.att + g.rel) as i64;
    // Control frames needed for interpolation over [a, b), plus smoothing context.
    let c_lo = div_floor(a, g.hop) - ctx;
    let c_hi = div_floor(b - 1, g.hop) + 1 + ctx;
    let raw: Vec<f32> = (c_lo..=c_hi)
        .map(|c| {
            let level = window_level_db(input, offset, clip_len, c * g.hop, g.win);
            knee_curve(level - r.threshold_dbfs, r.ratio, r.knee_db) as f32
        })
        .collect();
    let env = attack_release(&raw, g.att, g.rel);
    let at = |c: i64| env[(c - c_lo) as usize] as f64;
    let (s0, s1) = scope.samples(sr, clip_len);
    let ramp_len = (SCOPE_RAMP_S * sr as f64).round() as i64;
    let mut out = copy_window(target, offset, a, b);
    for ch in out.channels.iter_mut() {
        for (j, x) in ch.iter_mut().enumerate() {
            let i = a + j as i64;
            let c0 = div_floor(i, g.hop);
            let frac = (i - c0 * g.hop) as f64 / g.hop as f64;
            let (e0, e1) = (at(c0), at(c0 + 1));
            let red = if e0 == e1 { e0 } else { e0 + (e1 - e0) * frac };
            let w = scope_weight(i, s0 as i64, s1 as i64, clip_len as i64, ramp_len);
            let gain_db = (red + r.makeup_db) * w;
            if gain_db != 0.0 {
                *x = (*x as f64 * db_to_amp(gain_db)) as f32;
            }
        }
    }
    let used: Vec<f32> = (div_floor(a, g.hop)..=div_floor(b - 1, g.hop))
        .map(|c| at(c) as f32)
        .collect();
    (out, used)
}

impl Op for Compressor {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn measures_input(&self, params: &Value) -> bool {
        !params["makeup_db"].is_number()
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        typed::<Params>(params)?;
        if params["makeup_db"].is_number() {
            return Ok(params.clone());
        }
        // auto: the make-up gain that restores the input's peak.
        let mut probe = params.clone();
        probe["makeup_db"] = json!(0.0);
        let r: Resolved = typed(&probe)?;
        let (out, _) = render_with(
            &r,
            scope,
            input,
            input,
            0,
            0,
            input.len() as i64,
            input.len(),
        );
        let peak = |b: &AudioBuffer| {
            b.channels
                .iter()
                .flat_map(|c| c.iter())
                .fold(0.0f64, |m, &x| m.max((x as f64).abs()))
        };
        let (pin, pout) = (peak(input), peak(&out));
        let makeup = if pin > 0.0 && pout > 0.0 {
            (amp_to_db(pin) - amp_to_db(pout)).clamp(-24.0, 24.0)
        } else {
            0.0
        };
        Ok(with(params, json!({ "makeup_db": makeup })))
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        let Ok(r) = typed::<Resolved>(resolved) else {
            return 0;
        };
        let g = grid(&r, sr);
        (g.win / 2 + (g.att + g.rel + 2) as i64 * g.hop) as usize
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: Resolved = typed(resolved)?;
        let len = target.len();
        Ok(Some(
            render_with(&r, scope, decide, target, 0, 0, len as i64, len).0,
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
        let r: Resolved = typed(resolved)?;
        let (audio, env) = render_with(&r, scope, input, input, offset, a, b, clip_len);
        let active: Vec<f64> = env
            .iter()
            .filter(|&&v| v < 0.0)
            .map(|&v| v as f64)
            .collect();
        let mut m = Map::new();
        m.insert(
            "max_reduction_db".into(),
            json!(round_to(
                env.iter().fold(0.0f32, |x, &y| x.min(y)) as f64,
                2
            )),
        );
        m.insert(
            "mean_reduction_db".into(),
            json!(if active.is_empty() {
                0.0
            } else {
                round_to(active.iter().sum::<f64>() / active.len() as f64, 2)
            }),
        );
        m.insert("makeup_db".into(), json!(round_to(r.makeup_db, 2)));
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}
