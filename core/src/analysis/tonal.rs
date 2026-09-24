//! Tonal-line detection: narrow spectral lines that stand out from their
//! neighbourhood and persist through the analysed span.
//!
//! This automates the manual step "find the strongest lines by eye": the
//! long-term spectrum is measured at fine resolution (≈3 Hz bins at 48 kHz),
//! each local maximum's prominence is its level above the median of the
//! surrounding ±`neighbourhood_hz`, and its persistence is the share of ~1 s
//! segments in which it still stands out on its own.

use serde::{Deserialize, Serialize};

use super::spectrum::FramePower;
use crate::audio::AudioBuffer;
use crate::dsp::smooth::median_in_place;
use crate::dsp::stft::StftSize;
use crate::math::{power_to_db, round_to};

/// FFT size used for line analysis at 48 kHz (scaled with the sample rate).
pub const LINE_FFT_AT_48K: usize = 16384;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TonalLine {
    pub freq_hz: f64,
    /// Width at half the prominence (in dB) above the neighbourhood.
    pub width_hz: f64,
    /// Level above the median of the surrounding material, dB.
    pub prominence_db: f64,
    /// Share of ~1 s segments (0–1) in which the line stands out by ≥ 3 dB.
    pub persistence: f64,
    /// Long-term level at the line (dB relative to a full-scale sine).
    pub level_db: f64,
}

#[derive(Clone, Debug)]
pub struct LineConfig {
    pub f_lo: f64,
    pub f_hi: f64,
    pub neighbourhood_hz: f64,
    pub min_prominence_db: f64,
    pub min_persistence: f64,
    pub max_lines: usize,
}

impl Default for LineConfig {
    fn default() -> Self {
        LineConfig {
            f_lo: 20.0,
            f_hi: f64::INFINITY,
            neighbourhood_hz: 150.0,
            min_prominence_db: 6.0,
            min_persistence: 0.5,
            max_lines: 16,
        }
    }
}

/// Long-term spectrum at line resolution, kept so that line reductions can be
/// evaluated without re-analysing (a static mask scales it exactly).
pub struct LineSpectrum {
    pub size: StftSize,
    pub sample_rate: u32,
    pub ltas: Vec<f64>,
    pub a: usize,
    pub b: usize,
}

impl LineSpectrum {
    pub fn compute(audio: &AudioBuffer, a: usize, b: usize) -> Self {
        let size = StftSize::scaled(LINE_FFT_AT_48K, audio.sample_rate);
        let mut fp = FramePower::new(size, audio.sample_rate);
        let (k0, k1) = fp.frames_in(a, b);
        let mut ltas = vec![0.0f64; size.bins()];
        for k in k0..=k1 {
            for (l, &v) in ltas.iter_mut().zip(fp.compute(audio, k)) {
                *l += v;
            }
        }
        let frames = (k1 - k0 + 1) as f64;
        ltas.iter_mut().for_each(|l| *l /= frames);
        LineSpectrum {
            size,
            sample_rate: audio.sample_rate,
            ltas,
            a,
            b,
        }
    }

    pub fn bin_hz(&self) -> f64 {
        self.size.bin_hz(self.sample_rate)
    }

    pub fn db(&self) -> Vec<f64> {
        self.ltas.iter().map(|&p| power_to_db(p)).collect()
    }

    fn half_width_bins(&self, neighbourhood_hz: f64) -> usize {
        ((neighbourhood_hz / self.bin_hz()).round() as usize).max(4)
    }

    /// Median level of the neighbourhood around every bin.
    pub fn neighbourhood(db: &[f64], w: usize) -> Vec<f64> {
        let n = db.len();
        let mut buf: Vec<f32> = Vec::with_capacity(2 * w + 1);
        (0..n)
            .map(|b| {
                let lo = b.saturating_sub(w);
                let hi = (b + w + 1).min(n);
                buf.clear();
                buf.extend(db[lo..hi].iter().map(|&v| v as f32));
                median_in_place(&mut buf) as f64
            })
            .collect()
    }

    /// Prominence (dB above the neighbourhood median) of the strongest bin
    /// within ±2 bins of `freq_hz`, for a spectrum given in dB.
    pub fn prominence_at(&self, db: &[f64], neighbourhood: &[f64], freq_hz: f64) -> f64 {
        let c = (freq_hz / self.bin_hz()).round() as isize;
        (c - 2..=c + 2)
            .filter(|&b| b >= 0 && (b as usize) < db.len())
            .map(|b| db[b as usize] - neighbourhood[b as usize])
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// Detect lines in `audio[a..b]`. Persistence needs a second pass over the
/// frames, evaluated only at the candidate bins.
pub fn detect_lines(audio: &AudioBuffer, a: usize, b: usize, cfg: &LineConfig) -> Vec<TonalLine> {
    let spec = LineSpectrum::compute(audio, a, b);
    detect_in(&spec, audio, cfg)
}

pub fn detect_in(spec: &LineSpectrum, audio: &AudioBuffer, cfg: &LineConfig) -> Vec<TonalLine> {
    let bin_hz = spec.bin_hz();
    let nyq = spec.sample_rate as f64 / 2.0;
    let db = spec.db();
    let w = spec.half_width_bins(cfg.neighbourhood_hz);
    let hood = LineSpectrum::neighbourhood(&db, w);
    let n = db.len();
    let lo_bin = ((cfg.f_lo.max(20.0)) / bin_hz).ceil() as usize;
    let hi_bin = (((cfg.f_hi.min(0.95 * nyq)) / bin_hz).floor() as usize).min(n.saturating_sub(3));

    // Candidates: local maxima over ±2 bins standing ≥ 3 dB (or the requested
    // minimum, if lower) above their neighbourhood.
    let floor_prom = cfg.min_prominence_db.min(3.0);
    let mut cands: Vec<(usize, f64)> = Vec::new();
    for bi in lo_bin.max(2)..=hi_bin {
        let v = db[bi];
        let is_max = (bi - 2..=bi + 2).all(|j| j == bi || db[j] < v || (db[j] == v && j > bi));
        let prom = v - hood[bi];
        if is_max && prom >= floor_prom {
            cands.push((bi, prom));
        }
    }
    cands.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
    cands.truncate(64);

    // Persistence: in how many ~1 s segments does each candidate still stand
    // out by ≥ 3 dB? Averaging frames within a segment keeps noise from
    // producing false hits, which single frames do often.
    let mut fp = FramePower::new(spec.size, spec.sample_rate);
    let (k0, k1) = fp.frames_in(spec.a, spec.b);
    let total_frames = (k1 - k0 + 1) as usize;
    let per_segment = ((spec.sample_rate as f64 / spec.size.hop as f64).round() as usize).max(4);
    let segments = total_frames.div_ceil(per_segment).max(1);
    let mut hits = vec![0u32; cands.len()];
    let mut seg_acc = vec![0.0f64; n];
    let mut in_seg = 0usize;
    let mut buf: Vec<f32> = Vec::with_capacity(2 * w + 1);
    for (fi, k) in (k0..=k1).enumerate() {
        for (acc, &v) in seg_acc.iter_mut().zip(fp.compute(audio, k)) {
            *acc += v;
        }
        in_seg += 1;
        if in_seg == per_segment || fi + 1 == total_frames {
            for (ci, &(bi, _)) in cands.iter().enumerate() {
                let lo = bi.saturating_sub(w);
                let hi = (bi + w + 1).min(n);
                buf.clear();
                buf.extend(seg_acc[lo..hi].iter().map(|&v| power_to_db(v) as f32));
                let med = median_in_place(&mut buf) as f64;
                let peak = (bi - 1..=bi + 1)
                    .map(|j| power_to_db(seg_acc[j]))
                    .fold(f64::NEG_INFINITY, f64::max);
                if peak - med >= 3.0 {
                    hits[ci] += 1;
                }
            }
            seg_acc.iter_mut().for_each(|v| *v = 0.0);
            in_seg = 0;
        }
    }
    let frames = segments as f64;

    let mut lines: Vec<TonalLine> = Vec::new();
    for (ci, &(bi, prom)) in cands.iter().enumerate() {
        let persistence = hits[ci] as f64 / frames;
        if prom < cfg.min_prominence_db || persistence < cfg.min_persistence {
            continue;
        }
        // Parabolic refinement of the peak position.
        let (l, c, r) = (db[bi - 1], db[bi], db[bi + 1]);
        let denom = l - 2.0 * c + r;
        let delta = if denom != 0.0 {
            (0.5 * (l - r) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let freq = (bi as f64 + delta) * bin_hz;
        // Width at half the prominence.
        let half = hood[bi] + prom / 2.0;
        let mut left = bi;
        while left > 0 && db[left - 1] > half && bi - left < w {
            left -= 1;
        }
        let mut right = bi;
        while right + 1 < n && db[right + 1] > half && right - bi < w {
            right += 1;
        }
        let width = ((right - left + 1).max(2)) as f64 * bin_hz;
        if lines
            .iter()
            .any(|x| (x.freq_hz - freq).abs() < (x.width_hz.max(width)).max(3.0 * bin_hz))
        {
            continue; // a stronger line already covers this one
        }
        lines.push(TonalLine {
            freq_hz: round_to(freq, 2),
            width_hz: round_to(width, 2),
            prominence_db: round_to(prom, 2),
            persistence: round_to(persistence, 3),
            level_db: round_to(c, 2),
        });
        if lines.len() >= cfg.max_lines {
            break;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{sin, PI};

    fn noise(n: usize, seed: u64, amp: f64) -> Vec<f64> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn finds_steady_lines_and_ignores_intermittent_ones() {
        let sr = 48000u32;
        let len = sr as usize * 8;
        let nz = noise(len, 7, 0.01);
        let x: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f64 / sr as f64;
                let steady = 0.01 * sin(2.0 * PI * 750.0 * t) + 0.004 * sin(2.0 * PI * 3150.0 * t);
                let burst = if (2.0..3.0).contains(&t) {
                    0.05 * sin(2.0 * PI * 5200.0 * t)
                } else {
                    0.0
                };
                (nz[i] + steady + burst) as f32
            })
            .collect();
        let a = AudioBuffer::mono(sr, x);
        let lines = detect_lines(&a, 0, a.len(), &LineConfig::default());
        let f: Vec<f64> = lines.iter().map(|l| l.freq_hz).collect();
        assert!(f.iter().any(|&v| (v - 750.0).abs() < 1.0), "{lines:?}");
        assert!(f.iter().any(|&v| (v - 3150.0).abs() < 1.0), "{lines:?}");
        assert!(
            !f.iter().any(|&v| (v - 5200.0).abs() < 5.0),
            "intermittent whine is not a line: {lines:?}"
        );
        let l750 = lines
            .iter()
            .find(|l| (l.freq_hz - 750.0).abs() < 1.0)
            .unwrap();
        assert!(
            l750.prominence_db > 10.0 && l750.persistence > 0.9,
            "{l750:?}"
        );
    }
}
