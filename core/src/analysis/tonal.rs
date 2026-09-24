//! Tonal-line detection: narrow spectral lines that stand out from their
//! neighbourhood and persist through the analysed span.
//!
//! This automates the manual step "find the strongest lines by eye": the
//! long-term spectrum is measured at fine resolution (≈3 Hz bins at 48 kHz),
//! each local maximum's prominence is its level above the median of the
//! surrounding ±`neighbourhood_hz`, and its persistence is the share of ~1 s
//! segments in which it still stands out on its own.
//!
//! A line must also stand out in the speech pauses. Hum, whines and tones do
//! not stop when people stop talking; a voice harmonic that happens to recur
//! at one pitch does. Where a clip has too little pause to judge, a line must
//! instead be present almost throughout.

use serde::{Deserialize, Serialize};

use super::quiet;
use super::spectrum::FramePower;
use crate::audio::AudioBuffer;
use crate::dsp::smooth::median_in_place;
use crate::dsp::stft::{div_ceil, div_floor, StftSize};
use crate::math::{power_to_db, round_to};

/// FFT size used for line analysis at 48 kHz (scaled with the sample rate).
pub const LINE_FFT_AT_48K: usize = 16384;

/// A line frame is in a pause when at most this share of the 20 ms frames it
/// covers are active.
pub const PAUSE_MAX_ACTIVE_SHARE: f64 = 0.1;
/// Less pause than this in total (seconds) is not enough to judge a line by.
pub const MIN_PAUSE_S: f64 = 1.0;
/// How far a line must stand out in the pauses.
pub const PAUSE_MIN_PROMINENCE_DB: f64 = 3.0;
/// Without enough pause, the persistence a line needs instead.
pub const NO_PAUSE_MIN_PERSISTENCE: f64 = 0.9;

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
    /// Prominence in the speech pauses alone; absent when the span has less
    /// than `MIN_PAUSE_S` of pause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_prominence_db: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct LineConfig {
    pub f_lo: f64,
    pub f_hi: f64,
    pub neighbourhood_hz: f64,
    pub min_prominence_db: f64,
    pub min_persistence: f64,
    pub max_lines: usize,
    /// Broader peaks (formant regions, resonances) are not lines.
    pub max_width_hz: f64,
    /// Keep only lines that also stand out in the speech pauses (or, without
    /// enough pause, are present almost throughout).
    pub require_in_pauses: bool,
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
            max_width_hz: 25.0,
            require_in_pauses: true,
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

/// Where the speech pauses are, at the resolution of the 20 ms activity
/// frames (`quiet::frame_energies`, `quiet::active_frames`).
struct Pauses {
    /// Running count of active frames: `active[j]` frames before frame `j`.
    active: Vec<u32>,
    frame: usize,
    hop: usize,
    a: usize,
}

impl Pauses {
    fn new(audio: &AudioBuffer, a: usize, b: usize) -> Self {
        let (energies, frame, hop) = quiet::frame_energies(audio, a, b);
        let mut active = Vec::with_capacity(energies.len() + 1);
        active.push(0u32);
        for on in quiet::active_frames(&energies) {
            active.push(active.last().copied().unwrap_or(0) + on as u32);
        }
        Pauses {
            active,
            frame,
            hop,
            a,
        }
    }

    /// Whether samples `[s0, s1)` are a pause: at most `PAUSE_MAX_ACTIVE_SHARE`
    /// of the activity frames lying within them are active.
    fn is_pause(&self, s0: i64, s1: i64) -> bool {
        let n = self.active.len() as i64 - 1;
        let (hop, a) = (self.hop as i64, self.a as i64);
        let j0 = div_ceil((s0 - a).max(0), hop);
        let j1 = div_floor(s1 - a - self.frame as i64, hop).min(n - 1);
        if n <= 0 || j1 < j0 {
            return false;
        }
        let total = (j1 - j0 + 1) as f64;
        let on = (self.active[j1 as usize + 1] - self.active[j0 as usize]) as f64;
        on <= PAUSE_MAX_ACTIVE_SHARE * total
    }
}

/// Level of bin `bi` (the strongest within ±1 bin) above the median of ±`w` bins.
fn prominence_in(db: &[f64], bi: usize, w: usize, buf: &mut Vec<f32>) -> f64 {
    let n = db.len();
    let lo = bi.saturating_sub(w);
    let hi = (bi + w + 1).min(n);
    buf.clear();
    buf.extend(db[lo..hi].iter().map(|&v| v as f32));
    let med = median_in_place(buf) as f64;
    let peak = (bi.saturating_sub(1)..=(bi + 1).min(n - 1))
        .map(|j| db[j])
        .fold(f64::NEG_INFINITY, f64::max);
    peak - med
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
    // The same pass also averages the frames that lie in speech pauses.
    let pauses = Pauses::new(audio, spec.a, spec.b);
    let mut pause_acc = vec![0.0f64; n];
    let mut pause_frames = 0usize;
    for (fi, k) in (k0..=k1).enumerate() {
        let start = spec.size.frame_start(k);
        let in_pause = pauses.is_pause(
            start.max(spec.a as i64),
            (start + spec.size.n as i64).min(spec.b as i64),
        );
        let power = fp.compute(audio, k);
        for (acc, &v) in seg_acc.iter_mut().zip(power) {
            *acc += v;
        }
        if in_pause {
            for (acc, &v) in pause_acc.iter_mut().zip(power) {
                *acc += v;
            }
            pause_frames += 1;
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
    let pause_s = (pause_frames * spec.size.hop) as f64 / spec.sample_rate as f64;
    let pause_db: Option<Vec<f64>> = (pause_s >= MIN_PAUSE_S).then(|| {
        pause_acc
            .iter()
            .map(|&p| power_to_db(p / pause_frames as f64))
            .collect()
    });

    let mut lines: Vec<TonalLine> = Vec::new();
    for (ci, &(bi, prom)) in cands.iter().enumerate() {
        let persistence = hits[ci] as f64 / frames;
        if prom < cfg.min_prominence_db || persistence < cfg.min_persistence {
            continue;
        }
        let pause_prominence = pause_db.as_ref().map(|d| prominence_in(d, bi, w, &mut buf));
        if cfg.require_in_pauses {
            match pause_prominence {
                Some(p) if p < PAUSE_MIN_PROMINENCE_DB => continue,
                None if persistence < NO_PAUSE_MIN_PERSISTENCE => continue,
                _ => {}
            }
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
        if width > cfg.max_width_hz {
            continue;
        }
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
            pause_prominence_db: pause_prominence.map(|p| round_to(p, 2)),
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

    /// Speech-like bursts with a tone at a recurring pitch, and pauses of noise
    /// only; a steady line underneath throughout.
    fn talk_with_pauses(speech: impl Fn(f64) -> bool) -> AudioBuffer {
        let sr = 48000u32;
        let len = sr as usize * 12;
        let nz = noise(len, 11, 0.004);
        let burst = noise(len, 23, 0.03);
        let x: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f64 / sr as f64;
                let steady = 0.008 * sin(2.0 * PI * 750.0 * t);
                let voice = if speech(t) {
                    burst[i] + 0.04 * sin(2.0 * PI * 1860.0 * t)
                } else {
                    0.0
                };
                (nz[i] + steady + voice) as f32
            })
            .collect();
        AudioBuffer::mono(sr, x)
    }

    #[test]
    fn a_line_heard_only_while_someone_talks_is_not_a_line() {
        // Talking 70% of the time, in 1 s phrases with 0.43 s gaps and a long pause.
        let a = talk_with_pauses(|t| !(8.0..10.0).contains(&t) && (t % 1.43) < 1.0);
        let old = detect_lines(
            &a,
            0,
            a.len(),
            &LineConfig {
                require_in_pauses: false,
                ..LineConfig::default()
            },
        );
        assert!(
            old.iter().any(|l| (l.freq_hz - 1860.0).abs() < 2.0),
            "without the pause rule the recurring pitch passes: {old:?}"
        );
        let lines = detect_lines(&a, 0, a.len(), &LineConfig::default());
        let f: Vec<f64> = lines.iter().map(|l| l.freq_hz).collect();
        assert!(f.iter().any(|&v| (v - 750.0).abs() < 1.0), "{lines:?}");
        assert!(
            !f.iter().any(|&v| (v - 1860.0).abs() < 5.0),
            "only there while talking: {lines:?}"
        );
        let l750 = lines
            .iter()
            .find(|l| (l.freq_hz - 750.0).abs() < 1.0)
            .unwrap();
        assert!(
            l750.pause_prominence_db.unwrap() > 10.0,
            "the steady line stands out in the pauses: {l750:?}"
        );
    }

    #[test]
    fn without_pauses_a_line_must_be_present_almost_throughout() {
        // Talking almost all the time: gaps of 0.15 s set the floor, but no
        // line frame (0.34 s) falls in a pause, so there is no pause to judge by.
        let a = talk_with_pauses(|t| (t % 1.0) < 0.85);
        let lines = detect_lines(&a, 0, a.len(), &LineConfig::default());
        let l750 = lines.iter().find(|l| (l.freq_hz - 750.0).abs() < 1.0);
        assert!(
            l750.is_some_and(|l| l.pause_prominence_db.is_none()),
            "a steady line is still found, with no pause measurement: {lines:?}"
        );
    }

    #[test]
    fn the_short_golden_clip_has_exactly_its_real_lines() {
        let clip = crate::golden::generate(&crate::golden::spec_a_short());
        let lines = detect_lines(&clip.mix, 0, clip.mix.len(), &LineConfig::default());
        let mut f: Vec<i64> = lines.iter().map(|l| l.freq_hz.round() as i64).collect();
        f.sort();
        assert_eq!(f, vec![50, 100, 150, 750, 3150], "{lines:?}");
    }
}
