//! Sample-level statistics: peak, true peak, RMS, DC offset, clipping.

use crate::audio::AudioBuffer;
use crate::math::{amp_to_db, cos, sin, PI};

pub struct Levels {
    pub peak: f64,
    pub true_peak: f64,
    pub rms: f64,
    pub dc_offset: Vec<f64>,
    pub clipped_samples: u64,
}

pub fn levels(audio: &AudioBuffer, a: usize, b: usize) -> Levels {
    let n = (b - a).max(1) as f64;
    let mut peak = 0.0f64;
    let mut sumsq = 0.0f64;
    let mut dc = Vec::with_capacity(audio.num_channels());
    for ch in &audio.channels {
        let mut sum = 0.0f64;
        for &x in &ch[a..b] {
            let x = x as f64;
            sum += x;
            sumsq += x * x;
            peak = peak.max(x.abs());
        }
        dc.push(sum / n);
    }
    let rms = (sumsq / (n * audio.num_channels() as f64)).sqrt();
    let true_peak = audio
        .channels
        .iter()
        .map(|c| true_peak(&c[a..b]))
        .fold(peak, f64::max);
    let clipped_samples = audio.channels.iter().map(|c| clipped(&c[a..b], peak)).sum();
    Levels {
        peak,
        true_peak,
        rms,
        dc_offset: dc,
        clipped_samples,
    }
}

/// Samples in flattened peaks: runs of ≥ 3 identical non-zero samples whose
/// neighbours on both sides are smaller in magnitude. Recognises clipping at
/// any level, including clipping that happened upstream (a codec or AGC) and
/// was later attenuated, which a full-scale threshold would miss.
fn clipped(x: &[f32], _peak: f64) -> u64 {
    let mut count = 0u64;
    let mut i = 0;
    while i < x.len() {
        let v = x[i];
        let mut j = i + 1;
        while j < x.len() && x[j] == v {
            j += 1;
        }
        if j - i >= 3 && v != 0.0 {
            let before = if i > 0 {
                x[i - 1].abs() < v.abs()
            } else {
                false
            };
            let after = if j < x.len() {
                x[j].abs() < v.abs()
            } else {
                false
            };
            if before && after {
                count += (j - i) as u64;
            }
        }
        i = j;
    }
    count
}

/// 4× oversampled peak (windowed-sinc interpolation, 12 taps per phase).
fn true_peak(x: &[f32]) -> f64 {
    const PHASES: usize = 4;
    const TAPS: usize = 12;
    let half = TAPS as isize / 2;
    let mut kernel = [[0.0f64; TAPS]; PHASES];
    for (p, row) in kernel.iter_mut().enumerate() {
        let frac = p as f64 / PHASES as f64;
        for (t, k) in row.iter_mut().enumerate() {
            let d = (t as isize - half + 1) as f64 - frac;
            let sinc = if d == 0.0 {
                1.0
            } else {
                sin(PI * d) / (PI * d)
            };
            let w = 0.5 + 0.5 * cos(PI * d / (half as f64 + 1.0));
            *k = sinc * w;
        }
    }
    let mut peak = 0.0f64;
    let n = x.len() as isize;
    for i in 0..n {
        for row in kernel.iter().skip(1) {
            let mut acc = 0.0;
            for (t, k) in row.iter().enumerate() {
                let idx = i + t as isize - half + 1;
                if idx >= 0 && idx < n {
                    acc += x[idx as usize] as f64 * k;
                }
            }
            peak = peak.max(acc.abs());
        }
    }
    peak
}

pub fn db(x: f64) -> f64 {
    amp_to_db(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_levels() {
        let sr = 48000;
        let x: Vec<f32> = (0..sr)
            .map(|i| (0.5 * sin(2.0 * PI * 1000.0 * i as f64 / sr as f64)) as f32)
            .collect();
        let a = AudioBuffer::mono(sr as u32, x);
        let l = levels(&a, 0, a.len());
        assert!((db(l.peak) - db(0.5)).abs() < 0.01);
        assert!((db(l.rms) - (db(0.5) - 3.0103)).abs() < 0.01);
        assert!(l.true_peak >= l.peak && db(l.true_peak) - db(0.5) < 0.1);
        assert!(l.dc_offset[0].abs() < 1e-6);
        assert_eq!(l.clipped_samples, 0);
    }

    #[test]
    fn flattened_peaks_count_as_clipping() {
        let mut x = vec![0.0f32; 100];
        for v in x.iter_mut().skip(10).take(5) {
            *v = 0.03;
        }
        x[9] = 0.02;
        x[15] = 0.01;
        x[50] = 0.29;
        // A run of zeros (digital silence) and a plateau that is not a peak do not count.
        for v in x.iter_mut().skip(60).take(4) {
            *v = 0.05;
        }
        x[64] = 0.06;
        assert_eq!(clipped(&x, 0.3), 5);
    }
}
