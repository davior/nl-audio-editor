//! `hum_reduce`: cut the mains hum and its harmonics, each by a set depth,
//! whether or not a harmonic stands out on its own. The mains frequency is
//! found in the recording (`auto`) or stated (50 or 60 Hz), and measured from
//! the lines that are there, since the grid drifts a little from its nominal
//! value. Rendered as a static mask with `line_reduce`'s line shapes.
//!
//! `line_reduce` finds each line and cuts it until it matches its
//! surroundings; this cuts a fixed comb, which also reaches harmonics buried
//! under speech.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::mask::{self, Geometry, Reductions};
use super::spectral::{line_mask, CutLine};
use super::{typed, with, Descriptor, Op, OpError, RenderOut};
use crate::analysis::tonal::{self, LineConfig, LineSpectrum, TonalLine};
use crate::analysis::{detect_hum, hum_config};
use crate::audio::AudioBuffer;
use crate::dsp::stft::StftSize;
use crate::math::round_to;
use crate::scope::Scope;

pub struct HumReduce {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    fundamental: String,
    harmonics: i64,
    depth_db: f64,
    width_hz: f64,
}

#[derive(Deserialize)]
struct Resolved {
    fft_n: usize,
    resolved_lines: Vec<CutLine>,
}

/// The mains frequency measured from the lines near `base`'s harmonics (the
/// mean of each line's frequency over its harmonic number), or `base` itself
/// when none is there.
fn measured_fundamental(lines: &[TonalLine], base: f64) -> f64 {
    let ratios: Vec<f64> = (1..=6)
        .filter_map(|h| {
            let f = base * h as f64;
            lines
                .iter()
                .find(|l| (l.freq_hz - f).abs() <= 1.5)
                .map(|l| l.freq_hz / h as f64)
        })
        .collect();
    if ratios.is_empty() {
        base
    } else {
        ratios.iter().sum::<f64>() / ratios.len() as f64
    }
}

/// Fine enough that the cut at the fundamental spans four bins, so a
/// harmonic's energy falls inside its cut rather than beside it; never coarser
/// than the detection's, at most 65536 points at 48 kHz (scaled with the rate).
fn comb_size(width_hz: f64, detect: StftSize, sr: u32) -> StftSize {
    let max = StftSize::scaled(65536, sr).n.max(detect.n);
    let want = (4.0 * sr as f64 / width_hz).ceil() as usize;
    StftSize::new(want.next_power_of_two().clamp(detect.n, max))
}

/// Harmonic `k` is cut over this width: the mains frequency's drift is
/// multiplied at higher harmonics.
fn width_at(width_hz: f64, k: usize) -> f64 {
    width_hz * (1.0 + 0.1 * (k - 1) as f64)
}

impl Op for HumReduce {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: Params = typed(params)?;
        let sr = input.sample_rate;
        let (s0, s1) = scope.samples(sr, input.len());
        let spec = LineSpectrum::compute(input, s0, s1);
        let found = tonal::detect_in(&spec, input, &hum_config());
        let base = match p.fundamental.as_str() {
            "50" => Some(50.0),
            "60" => Some(60.0),
            _ => detect_hum(&found).map(|h| h.fundamental_hz),
        };
        // `auto` with no hum found cuts nothing, as line_reduce does with no lines.
        let f0 = base.map(|b| measured_fundamental(&found, b));
        let nyq = sr as f64 / 2.0;
        let lines: Vec<CutLine> = match f0 {
            None => Vec::new(),
            Some(f0) => (1..=p.harmonics as usize)
                .map(|k| (k, f0 * k as f64))
                .filter(|&(k, f)| f + width_at(p.width_hz, k) < nyq)
                .map(|(k, f)| CutLine {
                    freq_hz: round_to(f, 3),
                    width_hz: round_to(width_at(p.width_hz, k), 3),
                    depth_db: p.depth_db,
                    prominence_db: None,
                })
                .collect(),
        };
        Ok(with(
            params,
            json!({
                "fft_n": comb_size(p.width_hz, spec.size, sr).n,
                "fundamental_hz": f0.map(|f| round_to(f, 3)),
                "resolved_lines": lines,
            }),
        ))
    }

    fn radius(&self, resolved: &Value, _sr: u32) -> usize {
        typed::<Resolved>(resolved)
            .map(|r| mask::radius(StftSize::new(r.fft_n), 0))
            .unwrap_or(0)
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        // A static mask decides nothing from the signal.
        let r: Resolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let red = line_mask(&r.resolved_lines, 1.0, size, target.sample_rate);
        let len = target.len();
        let geom = Geometry::new(scope, size, target.sample_rate, len, false);
        let (audio, _) = mask::render(
            &geom,
            &Reductions::Static(&red),
            true,
            target,
            None,
            0,
            0,
            len as i64,
        );
        Ok(Some(audio))
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
        let sr = input.sample_rate;
        let red = line_mask(&r.resolved_lines, 1.0, size, sr);
        let geom = Geometry::new(scope, size, sr, clip_len, false);
        let (audio, st) = mask::render(
            &geom,
            &Reductions::Static(&red),
            true,
            input,
            None,
            offset,
            a,
            b,
        );
        let mut m = Map::new();
        m.insert("fundamental_hz".into(), resolved["fundamental_hz"].clone());
        m.insert("harmonics_cut".into(), json!(r.resolved_lines.len()));
        m.insert(
            "energy_removed_db".into(),
            json!(round_to(st.energy_removed_db(), 2)),
        );
        // How far each harmonic still stands out, as line_reduce reports it.
        let spec = LineSpectrum::compute(&audio, 0, audio.len());
        let db = spec.db();
        let w = ((LineConfig::default().neighbourhood_hz / spec.bin_hz()).round() as usize).max(4);
        let hood = LineSpectrum::neighbourhood(&db, w);
        let remaining = r
            .resolved_lines
            .iter()
            .map(|l| spec.prominence_at(&db, &hood, l.freq_hz).max(0.0))
            .fold(0.0f64, f64::max);
        m.insert(
            "max_remaining_prominence_db".into(),
            json!(round_to(remaining, 2)),
        );
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(freq_hz: f64) -> TonalLine {
        TonalLine {
            freq_hz,
            width_hz: 1.0,
            prominence_db: 10.0,
            persistence: 1.0,
            level_db: -60.0,
            pause_prominence_db: None,
        }
    }

    #[test]
    fn the_mains_frequency_is_measured_from_its_harmonics() {
        let lines = [line(49.95), line(99.9), line(149.85), line(750.0)];
        assert!((measured_fundamental(&lines, 50.0) - 49.95).abs() < 1e-9);
        assert_eq!(measured_fundamental(&[], 60.0), 60.0);
        assert!((width_at(3.0, 1) - 3.0).abs() < 1e-12);
        assert!((width_at(3.0, 11) - 6.0).abs() < 1e-12);
    }
}
