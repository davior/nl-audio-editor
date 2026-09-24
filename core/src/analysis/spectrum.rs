//! Long-term spectrum statistics: octave bands, noise floors, centroid, tilt,
//! bandwidth. Frames are processed one at a time so memory stays bounded on
//! long recordings.

use serde::{Deserialize, Serialize};

use crate::audio::AudioBuffer;
use crate::dsp::fft::RealFft;
use crate::dsp::smooth::quantile_in_place;
use crate::dsp::stft::StftSize;
use crate::dsp::window::hann_periodic;
use crate::math::{power_to_db, round_to};

/// Power spectrum of Hann-windowed frames, averaged over channels and scaled so
/// a full-scale sine reads 0 dB at its peak bin.
pub struct FramePower {
    pub size: StftSize,
    pub sample_rate: u32,
    window: Vec<f64>,
    norm: f64,
    fft: RealFft,
    frame: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
    acc: Vec<f64>,
}

impl FramePower {
    pub fn new(size: StftSize, sample_rate: u32) -> Self {
        let window = hann_periodic(size.n);
        let wsum: f64 = window.iter().sum();
        let norm = 1.0 / ((wsum / 2.0) * (wsum / 2.0));
        let bins = size.bins();
        FramePower {
            size,
            sample_rate,
            window,
            norm,
            fft: RealFft::new(size.n),
            frame: vec![0.0; size.n],
            re: vec![0.0; bins],
            im: vec![0.0; bins],
            acc: vec![0.0; bins],
        }
    }

    pub fn bins(&self) -> usize {
        self.size.bins()
    }

    pub fn bin_hz(&self) -> f64 {
        self.size.bin_hz(self.sample_rate)
    }

    /// Frames centred in `[a, b)`, at least one.
    pub fn frames_in(&self, a: usize, b: usize) -> (i64, i64) {
        let (k0, k1) = self.size.frames_centred_in(a as i64, b as i64);
        if k1 < k0 {
            let k = (a / self.size.hop) as i64;
            (k, k)
        } else {
            (k0, k1)
        }
    }

    /// Power spectrum of frame `k` of `audio`.
    pub fn compute(&mut self, audio: &AudioBuffer, k: i64) -> &[f64] {
        let start = self.size.frame_start(k);
        self.acc.iter_mut().for_each(|v| *v = 0.0);
        for ch in &audio.channels {
            for m in 0..self.size.n {
                let i = start + m as i64;
                let x = if i >= 0 && (i as usize) < ch.len() {
                    ch[i as usize] as f64
                } else {
                    0.0
                };
                self.frame[m] = x * self.window[m];
            }
            self.fft.forward(&self.frame, &mut self.re, &mut self.im);
            for b in 0..self.acc.len() {
                self.acc[b] += (self.re[b] * self.re[b] + self.im[b] * self.im[b]) * self.norm;
            }
        }
        let nch = audio.num_channels() as f64;
        for v in self.acc.iter_mut() {
            *v /= nch;
        }
        &self.acc
    }
}

/// Long-term average spectrum plus per-frame octave-band energies.
pub struct LongTerm {
    pub sample_rate: u32,
    pub bin_hz: f64,
    pub ltas: Vec<f64>,
    pub frames: usize,
    /// For each octave band: (centre, first bin, last bin).
    pub bands: Vec<(f64, usize, usize)>,
    /// For each band, the energy (dB) of every frame.
    pub band_frames: Vec<Vec<f32>>,
}

pub const OCTAVE_CENTRES: [f64; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

pub fn long_term(audio: &AudioBuffer, a: usize, b: usize, size: StftSize) -> LongTerm {
    let mut fp = FramePower::new(size, audio.sample_rate);
    let bins = fp.bins();
    let bin_hz = fp.bin_hz();
    let nyq = audio.sample_rate as f64 / 2.0;
    let mut bands = Vec::new();
    for &c in OCTAVE_CENTRES.iter() {
        if c * core::f64::consts::SQRT_2 > nyq * 1.02 {
            break;
        }
        let lo = ((c / core::f64::consts::SQRT_2) / bin_hz).ceil().max(1.0) as usize;
        let hi = (((c * core::f64::consts::SQRT_2) / bin_hz).floor() as usize).min(bins - 1);
        if hi >= lo {
            bands.push((c, lo, hi));
        }
    }
    let (k0, k1) = fp.frames_in(a, b);
    let mut ltas = vec![0.0f64; bins];
    let mut band_frames = vec![Vec::with_capacity((k1 - k0 + 1) as usize); bands.len()];
    for k in k0..=k1 {
        let p = fp.compute(audio, k);
        for (l, &v) in ltas.iter_mut().zip(p) {
            *l += v;
        }
        for (bi, &(_, lo, hi)) in bands.iter().enumerate() {
            band_frames[bi].push(power_to_db(p[lo..=hi].iter().sum()) as f32);
        }
    }
    let frames = (k1 - k0 + 1) as usize;
    for l in ltas.iter_mut() {
        *l /= frames as f64;
    }
    LongTerm {
        sample_rate: audio.sample_rate,
        bin_hz,
        ltas,
        frames,
        bands,
        band_frames,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BandStat {
    pub center_hz: f64,
    /// Mean energy in the band (dB relative to a full-scale sine).
    pub energy_db: f64,
    /// 10th-percentile frame energy in the band: the band's noise floor.
    pub floor_db: f64,
}

pub fn octave_bands(lt: &LongTerm) -> Vec<BandStat> {
    lt.bands
        .iter()
        .zip(&lt.band_frames)
        .map(|(&(c, lo, hi), frames)| {
            let energy: f64 = lt.ltas[lo..=hi].iter().sum();
            let mut f = frames.clone();
            let floor = quantile_in_place(&mut f, 0.1) as f64;
            BandStat {
                center_hz: c,
                energy_db: round_to(power_to_db(energy), 2),
                floor_db: round_to(floor, 2),
            }
        })
        .collect()
}

pub fn centroid_hz(ltas: &[f64], bin_hz: f64) -> f64 {
    let total: f64 = ltas.iter().skip(1).sum();
    if total <= 0.0 {
        return 0.0;
    }
    ltas.iter()
        .enumerate()
        .skip(1)
        .map(|(b, &p)| b as f64 * bin_hz * p)
        .sum::<f64>()
        / total
}

/// Least-squares slope of octave-band energy against octave number (dB/octave),
/// over bands from 63 Hz up.
pub fn tilt_db_per_octave(bands: &[BandStat]) -> f64 {
    let pts: Vec<(f64, f64)> = bands
        .iter()
        .filter(|b| b.center_hz >= 63.0 && b.energy_db > crate::math::DB_FLOOR + 1.0)
        .map(|b| (crate::math::log2(b.center_hz), b.energy_db))
        .collect();
    if pts.len() < 2 {
        return 0.0;
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    if sxx == 0.0 {
        0.0
    } else {
        sxy / sxx
    }
}

/// Upper band edge: the highest frequency (above 2 kHz) where the long-term
/// spectrum falls by ≥ 20 dB within a sixth of an octave — the cliff a codec's
/// low-pass or a band-limited channel leaves. Nyquist if there is no cliff.
pub fn bandwidth_hz(ltas: &[f64], bin_hz: f64, sample_rate: u32) -> (f64, bool) {
    let nyq = sample_rate as f64 / 2.0;
    let db: Vec<f64> = ltas.iter().map(|&p| power_to_db(p)).collect();
    let mean_db = |f_lo: f64, f_hi: f64| -> Option<f64> {
        let lo = (f_lo / bin_hz).ceil() as usize;
        let hi = ((f_hi / bin_hz).floor() as usize).min(db.len() - 1);
        if hi < lo {
            return None;
        }
        Some(db[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64)
    };
    let step = crate::math::pow(2.0, 1.0 / 12.0);
    let sixth = crate::math::pow(2.0, 1.0 / 6.0);
    let mut edge = None;
    let mut f = 2000.0;
    while f * sixth < nyq * 0.99 {
        if let (Some(below), Some(above)) = (mean_db(f / sixth, f), mean_db(f, f * sixth)) {
            if below - above >= 20.0 {
                edge = Some(f);
            }
        }
        f *= step;
    }
    match edge {
        Some(e) => (round_to(e, 0), true),
        None => (nyq, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{sin, PI};

    #[test]
    fn full_scale_sine_reads_near_zero_db() {
        let sr = 48000u32;
        let x: Vec<f32> = (0..sr)
            .map(|i| sin(2.0 * PI * 1000.0 * i as f64 / sr as f64) as f32)
            .collect();
        let a = AudioBuffer::mono(sr, x);
        let lt = long_term(&a, 0, a.len(), StftSize::new(2048));
        let peak = lt.ltas.iter().cloned().fold(0.0, f64::max);
        // 1000 Hz falls between bins (23.4 Hz spacing): allow Hann scalloping.
        assert!(
            power_to_db(peak) > -1.5 && power_to_db(peak) < 0.1,
            "{}",
            power_to_db(peak)
        );
        let c = centroid_hz(&lt.ltas, lt.bin_hz);
        assert!((c - 1000.0).abs() < 30.0, "{c}");
        let (bw, limited) = bandwidth_hz(&lt.ltas, lt.bin_hz, sr);
        assert!(!limited && bw == 24000.0);
    }
}
