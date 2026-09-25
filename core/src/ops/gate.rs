//! `gate`: lower the level where it falls below a threshold (the pauses
//! between words) by up to a set range.
//!
//! The level is measured on the compressor's control grid: a 10 ms RMS every
//! ~0.67 ms, anchored to the clip start, each window summed from scratch.
//! How open the gate is spreads with finite reach: it opens `attack_ms` ahead
//! of the level rising, so onsets are kept, stays open `hold_ms` after the
//! level falls, then closes over `release_ms`. It only ever reduces.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::compressor::{control_grid, window_level_db};
use super::level::{scope_weight, SCOPE_RAMP_S};
use super::{copy_window, typed, with, Descriptor, Op, OpError, RenderOut};
use crate::audio::AudioBuffer;
use crate::dsp::smooth::{attack_release, quantile_in_place, trailing_max};
use crate::dsp::stft::div_floor;
use crate::math::{db_to_amp, power_to_db, round_to, DB_FLOOR};
use crate::scope::Scope;

/// Fully open at the threshold, fully closed this far below it.
const KNEE_DB: f64 = 3.0;
/// `auto` puts the threshold this far above the scope's noise floor.
const AUTO_MARGIN_DB: f64 = 6.0;

pub struct Gate {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // parsed to check the parameter set against the descriptor
struct Params {
    threshold_dbfs: Value,
    range_db: f64,
    attack_ms: f64,
    hold_ms: f64,
    release_ms: f64,
}

#[derive(Deserialize)]
struct Resolved {
    threshold_dbfs: f64,
    range_db: f64,
    attack_ms: f64,
    hold_ms: f64,
    release_ms: f64,
}

struct Frames {
    hop: i64,
    win: i64,
    att: usize,
    hold: usize,
    rel: usize,
}

fn frames(r: &Resolved, sr: u32) -> Frames {
    let (hop, win) = control_grid(sr);
    let n = |ms: f64| ((ms / 1000.0 * sr as f64) / hop as f64).round() as usize;
    Frames {
        hop,
        win,
        att: n(r.attack_ms),
        hold: n(r.hold_ms),
        rel: n(r.release_ms),
    }
}

/// The noise floor of a stretch: the 10th percentile of its 10 ms levels,
/// taken back to back (dB).
fn noise_floor_db(input: &AudioBuffer, s0: usize, s1: usize) -> f64 {
    let (_, win) = control_grid(input.sample_rate);
    let win = win as usize;
    let nch = input.num_channels();
    let mut levels = Vec::new();
    let mut i = s0;
    while i + win <= s1 {
        let acc: f64 = input
            .channels
            .iter()
            .map(|c| {
                c[i..i + win]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>()
            })
            .sum();
        levels.push(power_to_db(acc / (win * nch) as f64) as f32);
        i += win;
    }
    if levels.is_empty() {
        DB_FLOOR
    } else {
        quantile_in_place(&mut levels, 0.1) as f64
    }
}

/// The gate applied to `target` with its opening decided from `input`
/// (normally the same buffer); also the reduction at each control frame
/// that falls in `[a, b)`.
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
    let g = frames(r, sr);
    let ctx = (g.att + g.hold + g.rel) as i64;
    let c_lo = div_floor(a, g.hop) - ctx;
    let c_hi = div_floor(b - 1, g.hop) + 1 + ctx;
    // How open (0 = closed … range = open) each frame's level makes the gate.
    let open: Vec<f32> = (c_lo..=c_hi)
        .map(|c| {
            let level = window_level_db(input, offset, clip_len, c * g.hop, g.win);
            let x = ((level - r.threshold_dbfs) / KNEE_DB + 1.0).clamp(0.0, 1.0);
            (r.range_db * x) as f32
        })
        .collect();
    // Held open after the level falls, then opened ahead of onsets and closed
    // over the release: attack_release spreads reductions, so it is given
    // the opening negated.
    let held: Vec<f32> = trailing_max(&open, g.hold).iter().map(|&o| -o).collect();
    let env: Vec<f32> = attack_release(&held, g.att, g.rel)
        .iter()
        .map(|&s| -s - r.range_db as f32)
        .collect();
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
            if red == 0.0 {
                continue;
            }
            let w = scope_weight(i, s0 as i64, s1 as i64, clip_len as i64, ramp_len);
            let gain_db = red * w;
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

impl Op for Gate {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        typed::<Params>(params)?;
        if params["threshold_dbfs"].is_number() {
            return Ok(params.clone());
        }
        // auto: a margin above the scope's noise floor.
        let (s0, s1) = scope.samples(input.sample_rate, input.len());
        let floor = noise_floor_db(input, s0, s1);
        let threshold = round_to((floor + AUTO_MARGIN_DB).clamp(-100.0, 0.0), 2);
        Ok(with(
            params,
            json!({ "threshold_dbfs": threshold, "noise_floor_dbfs": round_to(floor.max(DB_FLOOR), 2) }),
        ))
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        let Ok(r) = typed::<Resolved>(resolved) else {
            return 0;
        };
        let g = frames(&r, sr);
        (g.win / 2 + (g.att + g.hold + g.rel + 2) as i64 * g.hop) as usize
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
        let before = copy_window(input, offset, a, b);
        let (mut energy, mut removed) = (0.0f64, 0.0f64);
        for (x, y) in before.channels.iter().zip(&audio.channels) {
            for (&p, &q) in x.iter().zip(y) {
                energy += (p as f64) * (p as f64);
                removed += (p as f64 - q as f64) * (p as f64 - q as f64);
            }
        }
        let closed = env.iter().filter(|&&v| v < -1.0).count();
        let mut m = Map::new();
        m.insert(
            "threshold_dbfs".into(),
            json!(round_to(r.threshold_dbfs, 2)),
        );
        m.insert(
            "closed_pct".into(),
            json!(round_to(100.0 * closed as f64 / env.len().max(1) as f64, 1)),
        );
        m.insert(
            "max_reduction_db".into(),
            json!(round_to(
                env.iter().fold(0.0f32, |x, &y| x.min(y)) as f64,
                2
            )),
        );
        m.insert(
            "energy_removed_db".into(),
            json!(if energy > 0.0 && removed > 0.0 {
                round_to(power_to_db(removed / energy), 2)
            } else {
                DB_FLOOR
            }),
        );
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}
