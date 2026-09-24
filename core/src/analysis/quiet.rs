//! The quietest steady region: where a noise profile should be taken.
//!
//! Encodes the manual rule "take the quietest part of the clip, using the
//! shortest sample that will work". For each candidate length (shortest
//! first) the quietest window is found; it "works" if its two halves have the
//! same spectrum within 1 dB (median absolute difference per bin), i.e. it is
//! steady noise rather than the edge of speech. Digital silence is skipped.

use serde::{Deserialize, Serialize};

use super::spectrum::FramePower;
use crate::audio::AudioBuffer;
use crate::dsp::smooth::median_in_place;
use crate::dsp::stft::StftSize;
use crate::math::{power_to_db, round_to};

pub const CANDIDATE_LENGTHS_S: [f64; 5] = [0.25, 0.5, 1.0, 1.5, 2.0];
const FRAME_S: f64 = 0.02;
const HOP_S: f64 = 0.01;
const SILENCE_DB: f64 = -150.0;
const STABLE_DB: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct QuietRegion {
    pub t0: f64,
    pub t1: f64,
    /// Mean level over the region, dBFS (RMS).
    pub level_dbfs: f64,
    /// Whether the two halves agreed within 1 dB.
    pub stable: bool,
    /// Median absolute per-bin difference between the halves, dB.
    pub halves_diff_db: f64,
}

/// Frame energies (dB): AC RMS (each frame's mean removed, so a DC offset does
/// not masquerade as noise) over 20 ms frames every 10 ms, averaged over channels.
pub fn frame_energies(audio: &AudioBuffer, a: usize, b: usize) -> (Vec<f64>, usize, usize) {
    let sr = audio.sample_rate as f64;
    let frame = ((FRAME_S * sr).round() as usize).max(1);
    let hop = ((HOP_S * sr).round() as usize).max(1);
    let mut out = Vec::new();
    let mut s = a;
    while s + frame <= b {
        let mut acc = 0.0;
        for ch in &audio.channels {
            let x = &ch[s..s + frame];
            let mean = x.iter().map(|&v| v as f64).sum::<f64>() / frame as f64;
            acc += x
                .iter()
                .map(|&v| (v as f64 - mean) * (v as f64 - mean))
                .sum::<f64>();
        }
        out.push(power_to_db(acc / (frame * audio.num_channels()) as f64));
        s += hop;
    }
    (out, frame, hop)
}

pub fn quietest_region(audio: &AudioBuffer, a: usize, b: usize) -> Option<QuietRegion> {
    let (e, frame, hop) = frame_energies(audio, a, b);
    if e.is_empty() {
        return None;
    }
    let sr = audio.sample_rate as f64;
    let power: Vec<f64> = e.iter().map(|&d| crate::math::db_to_power(d)).collect();
    let mut fallback: Option<QuietRegion> = None;
    for &len_s in CANDIDATE_LENGTHS_S.iter() {
        let m = (((len_s * sr) as usize).saturating_sub(frame) / hop + 1).max(1);
        if m > e.len() {
            break;
        }
        // Quietest window of m frames with no digitally silent frame.
        let mut best: Option<(usize, f64)> = None;
        let mut sum: f64 = power[..m].iter().sum();
        let mut silent = e[..m].iter().filter(|&&d| d < SILENCE_DB).count();
        for start in 0..=e.len() - m {
            if start > 0 {
                sum += power[start + m - 1] - power[start - 1];
                if e[start + m - 1] < SILENCE_DB {
                    silent += 1;
                }
                if e[start - 1] < SILENCE_DB {
                    silent -= 1;
                }
            }
            if silent == 0 {
                let mean = sum / m as f64;
                if best.map_or(true, |(_, v)| mean < v) {
                    best = Some((start, mean));
                }
            }
        }
        let Some((start, mean)) = best else { continue };
        let s0 = a + start * hop;
        let s1 = (s0 + (len_s * sr).round() as usize).min(b);
        let diff = halves_difference(audio, s0, s1);
        let region = QuietRegion {
            t0: round_to(s0 as f64 / sr, 3),
            t1: round_to(s1 as f64 / sr, 3),
            level_dbfs: round_to(power_to_db(mean.max(0.0)), 2),
            stable: diff <= STABLE_DB,
            halves_diff_db: round_to(diff, 2),
        };
        if region.stable {
            return Some(region);
        }
        fallback = Some(region);
    }
    fallback
}

/// Median over bins (100 Hz – min(8 kHz, 0.9·Nyquist)) of the absolute dB
/// difference between the average spectra of the two halves of `[s0, s1)`.
fn halves_difference(audio: &AudioBuffer, s0: usize, s1: usize) -> f64 {
    let size = StftSize::scaled(2048, audio.sample_rate);
    let mut fp = FramePower::new(size, audio.sample_rate);
    let mid = s0 + (s1 - s0) / 2;
    let mut avg = |x0: usize, x1: usize| -> Vec<f64> {
        let (k0, k1) = fp.frames_in(
            x0 + size.n / 2,
            x1.saturating_sub(size.n / 2).max(x0 + size.n / 2 + 1),
        );
        let mut acc = vec![0.0f64; size.bins()];
        for k in k0..=k1 {
            for (a, &p) in acc.iter_mut().zip(fp.compute(audio, k)) {
                *a += p;
            }
        }
        acc
    };
    let h1 = avg(s0, mid);
    let h2 = avg(mid, s1);
    let bin_hz = size.bin_hz(audio.sample_rate);
    let lo = (100.0 / bin_hz).ceil() as usize;
    let hi = ((8000.0f64.min(0.45 * audio.sample_rate as f64) / bin_hz).floor() as usize)
        .min(size.bins() - 1);
    let mut d: Vec<f32> = (lo..=hi)
        .filter(|&b| h1[b] > 0.0 && h2[b] > 0.0)
        .map(|b| (power_to_db(h1[b]) - power_to_db(h2[b])).abs() as f32)
        .collect();
    if d.is_empty() {
        return f64::INFINITY;
    }
    median_in_place(&mut d) as f64
}

/// A 20 ms frame is active (speech, or any other foreground sound) when its
/// energy is more than this far above the floor.
pub const ACTIVE_ABOVE_FLOOR_DB: f64 = 9.0;

/// The floor activity is measured against: the 10th percentile of the frame
/// energies.
pub fn activity_floor(energies: &[f64]) -> f64 {
    if energies.is_empty() {
        return crate::math::DB_FLOOR;
    }
    let mut v: Vec<f32> = energies.iter().map(|&d| d as f32).collect();
    crate::dsp::smooth::quantile_in_place(&mut v, 0.1) as f64
}

/// Which frames are active, by the rule above.
pub fn active_frames(energies: &[f64]) -> Vec<bool> {
    let floor = activity_floor(energies);
    energies
        .iter()
        .map(|&d| d > floor + ACTIVE_ABOVE_FLOOR_DB)
        .collect()
}

/// Share of 20 ms frames more than 9 dB above the noise floor (10th percentile).
pub fn activity_ratio(energies: &[f64]) -> f64 {
    if energies.is_empty() {
        return 0.0;
    }
    let active = active_frames(energies).iter().filter(|&&a| a).count();
    round_to(active as f64 / energies.len() as f64, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_quiet_gap_and_prefers_a_short_stable_window() {
        let sr = 48000u32;
        let mut s = 99u64;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        let len = sr as usize * 6;
        let x: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f64 / sr as f64;
                let loud = if (2.0..3.5).contains(&t) { 0.001 } else { 0.05 };
                (rnd() * loud) as f32
            })
            .collect();
        let a = AudioBuffer::mono(sr, x);
        let q = quietest_region(&a, 0, a.len()).unwrap();
        assert!(q.stable, "{q:?}");
        assert!(q.t0 >= 2.0 && q.t1 <= 3.5, "{q:?}");
        assert!(q.t1 - q.t0 <= 1.0, "prefers a short window: {q:?}");
    }
}
