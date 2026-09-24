//! Noise reduction: a spectral gate against a per-bin noise profile.
//!
//! The profile is the average spectrum of a quiet region (by default the
//! quietest steady region — "the quietest part, using the shortest sample that
//! will work"). A cell no louder than the profile plus `sensitivity_db` is
//! reduced by `reduction_db`. Openings (kept cells) spread a little before
//! (attack, look-ahead) and after (release) so speech edges are protected, and
//! are smoothed across `frequency_smoothing_bands` bins on each side.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::mask::{self, Geometry, Levels, Reductions};
use super::{typed, with, Descriptor, Op, OpError, RenderOut};
use crate::analysis::quiet::quietest_region;
use crate::audio::AudioBuffer;
use crate::dsp::smooth::attack_release;
use crate::dsp::stft::StftSize;
use crate::math::{power_to_db, round_to};
use crate::scope::Scope;

pub const NR_FFT_AT_48K: usize = 2048;

pub struct NoiseReduce {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // parsed to check the parameter set against the descriptor
struct Params {
    profile: Value,
    reduction_db: f64,
    sensitivity_db: f64,
    frequency_smoothing_bands: i64,
    attack_ms: f64,
    release_ms: f64,
}

#[derive(Deserialize)]
struct Resolved {
    reduction_db: f64,
    sensitivity_db: f64,
    frequency_smoothing_bands: i64,
    attack_ms: f64,
    release_ms: f64,
    fft_n: usize,
    profile_db: Vec<f64>,
}

fn frames(ms: f64, size: StftSize, sr: u32) -> usize {
    ((ms / 1000.0 * sr as f64) / size.hop as f64).round() as usize
}

impl NoiseReduce {
    fn reductions(
        r: &Resolved,
        size: StftSize,
        sr: u32,
    ) -> (usize, impl Fn(&Levels, i64, i64) -> Vec<f32> + '_) {
        let att = frames(r.attack_ms, size, sr);
        let rel = frames(r.release_ms, size, sr);
        let smooth = r.frequency_smoothing_bands.max(0) as usize;
        let reduction = r.reduction_db as f32;
        let thresholds: Vec<f32> = r
            .profile_db
            .iter()
            .map(|&p| (p + r.sensitivity_db) as f32)
            .collect();
        let context = att + rel;
        let compute = move |lv: &Levels, k0: i64, k1: i64| -> Vec<f32> {
            let bins = lv.bins;
            let out_frames = (k1 - k0 + 1) as usize;
            let mut out = vec![0.0f32; out_frames * bins];
            if reduction == 0.0 {
                return out;
            }
            // Openness per bin over the context range: −R where the cell is kept.
            let lo = k0 - rel as i64;
            let hi = k1 + att as i64;
            let span = (hi - lo + 1) as usize;
            let mut open = vec![vec![0.0f32; span]; bins];
            for (fi, k) in (lo..=hi).enumerate() {
                if !lv.contains(k) {
                    continue;
                }
                for b in 0..bins {
                    if lv.at(k, b) >= thresholds[b] {
                        open[b][fi] = -reduction;
                    }
                }
            }
            let spread: Vec<Vec<f32>> = open.iter().map(|o| attack_release(o, att, rel)).collect();
            for fi in 0..out_frames {
                let si = fi + rel;
                for b in 0..bins {
                    let a = b.saturating_sub(smooth);
                    let z = (b + smooth + 1).min(bins);
                    let mean =
                        spread[a..z].iter().map(|s| -s[si] as f64).sum::<f64>() / (z - a) as f64;
                    // Openness u ∈ [0, R]; reduction = u − R.
                    out[fi * bins + b] = (mean as f32 - reduction).min(0.0);
                }
            }
            out
        };
        (context, compute)
    }
}

impl Op for NoiseReduce {
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
        let sr = input.sample_rate;
        let (t0, t1, auto) = if params["profile"].as_str() == Some("auto") {
            let (s0, s1) = scope.samples(sr, input.len());
            let q = quietest_region(input, s0, s1).ok_or_else(|| {
                OpError::Invalid("no usable noise profile: the clip is silent or too short".into())
            })?;
            (q.t0, q.t1, true)
        } else {
            let t0 = params["profile"]["t0"].as_f64().unwrap_or(0.0);
            let t1 = params["profile"]["t1"].as_f64().unwrap_or(0.0);
            if t1 > input.duration_s() + 1e-9 {
                return Err(OpError::Invalid(format!(
                    "noise profile {t0}–{t1} s runs past the end of the clip"
                )));
            }
            (t0, t1, false)
        };
        let size = StftSize::scaled(NR_FFT_AT_48K, sr);
        let (p0, p1) = (
            input.time_to_sample(t0) as i64,
            input.time_to_sample(t1) as i64,
        );
        let (mut k0, mut k1) = size.frames_centred_in(p0, p1);
        if k1 < k0 {
            k0 = p0 / size.hop as i64;
            k1 = k0;
        }
        let lv = mask::levels(size, input, 0, input.len(), k0, k1, None);
        let bins = size.bins();
        let mut acc = vec![0.0f64; bins];
        for k in k0..=k1 {
            for (b, a) in acc.iter_mut().enumerate() {
                *a += crate::math::db_to_power(lv.at(k, b) as f64);
            }
        }
        let nf = (k1 - k0 + 1) as f64;
        let profile_db: Vec<f64> = acc
            .iter()
            .map(|&p| round_to(power_to_db(p / nf), 2))
            .collect();
        Ok(with(
            params,
            json!({
                "profile_range": { "t0": t0, "t1": t1, "auto": auto },
                "fft_n": size.n,
                "profile_db": profile_db,
            }),
        ))
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        let Ok(r) = typed::<Resolved>(resolved) else {
            return 0;
        };
        let size = StftSize::new(r.fft_n);
        mask::radius(
            size,
            frames(r.attack_ms, size, sr) + frames(r.release_ms, size, sr),
        )
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
        let size = StftSize::new(r.fft_n);
        if r.profile_db.len() != size.bins() {
            return Err(OpError::Invalid(
                "noise profile does not match the analysis size".into(),
            ));
        }
        let geom = Geometry::new(scope, size, input.sample_rate, clip_len, true);
        let (context, compute) = Self::reductions(&r, size, input.sample_rate);
        let (audio, st) = mask::render(
            &geom,
            &Reductions::Dynamic {
                context,
                compute: &compute,
            },
            true,
            input,
            None,
            offset,
            a,
            b,
        );
        let mut m = Map::new();
        m.insert(
            "cells_reduced_pct".into(),
            json!(round_to(st.cells_reduced_pct(), 2)),
        );
        m.insert(
            "mean_reduction_db".into(),
            json!(round_to(st.mean_reduction_db(), 2)),
        );
        m.insert(
            "energy_removed_db".into(),
            json!(round_to(st.energy_removed_db(), 2)),
        );
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: Resolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let len = target.len();
        let geom = Geometry::new(scope, size, target.sample_rate, len, true);
        let (context, compute) = Self::reductions(&r, size, target.sample_rate);
        let reductions = Reductions::Dynamic {
            context,
            compute: &compute,
        };
        let (audio, _) = mask::render(
            &geom,
            &reductions,
            true,
            target,
            Some(decide),
            0,
            0,
            len as i64,
        );
        Ok(Some(audio))
    }
}
