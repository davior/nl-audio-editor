//! Short-time Fourier transform on a frame grid anchored to the clip start.
//!
//! Frame `k` covers absolute samples `[k·hop − n/2, k·hop + n/2)`, so its centre
//! is at `k·hop` whatever part of the clip is being processed. Anchoring the grid
//! this way is what makes a preview of a window bit-identical to the same span
//! of a full render. Analysis and synthesis both use a periodic sqrt-Hann window
//! at 75% overlap (`hop = n/4`), whose squared sum is exactly 2.

use super::fft::RealFft;
use super::window::sqrt_hann_periodic;

/// Squared-window sum at `hop = n/4` for the sqrt-Hann pair.
pub const WOLA_GAIN: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StftSize {
    pub n: usize,
    pub hop: usize,
}

impl StftSize {
    pub fn new(n: usize) -> Self {
        assert!(n >= 16 && n.is_power_of_two());
        StftSize { n, hop: n / 4 }
    }

    /// FFT size scaled from a reference size at 48 kHz to `sample_rate`, rounded
    /// to the nearest power of two so bins keep roughly the same width in Hz.
    pub fn scaled(n_at_48k: usize, sample_rate: u32) -> Self {
        let target = n_at_48k as f64 * sample_rate as f64 / 48000.0;
        let mut n = 16usize;
        while (n * 2) as f64 <= target * core::f64::consts::SQRT_2 {
            n *= 2;
        }
        StftSize::new(n)
    }

    pub fn bins(&self) -> usize {
        self.n / 2 + 1
    }

    pub fn bin_hz(&self, sample_rate: u32) -> f64 {
        sample_rate as f64 / self.n as f64
    }

    /// First sample (absolute) covered by frame `k`.
    pub fn frame_start(&self, k: i64) -> i64 {
        k * self.hop as i64 - (self.n / 2) as i64
    }

    /// Frames whose extent overlaps absolute samples `[a, b)`.
    pub fn frames_overlapping(&self, a: i64, b: i64) -> (i64, i64) {
        let hop = self.hop as i64;
        let half = (self.n / 2) as i64;
        // k·hop − half < b  and  k·hop + half > a
        let k_lo = div_floor(a - half, hop) + 1;
        let k_hi = div_ceil(b + half, hop) - 1;
        (k_lo, k_hi)
    }

    /// Frames whose centre `k·hop` lies in `[a, b)`.
    pub fn frames_centred_in(&self, a: i64, b: i64) -> (i64, i64) {
        let hop = self.hop as i64;
        (div_ceil(a, hop), div_ceil(b, hop) - 1)
    }
}

pub fn div_floor(a: i64, b: i64) -> i64 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

pub fn div_ceil(a: i64, b: i64) -> i64 {
    -div_floor(-a, b)
}

/// Reusable analysis/synthesis state for one STFT size.
pub struct Stft {
    pub size: StftSize,
    pub window: Vec<f64>,
    fft: RealFft,
    frame: Vec<f64>,
    pub re: Vec<f64>,
    pub im: Vec<f64>,
}

impl Stft {
    pub fn new(size: StftSize) -> Self {
        Stft {
            size,
            window: sqrt_hann_periodic(size.n),
            fft: RealFft::new(size.n),
            frame: vec![0.0; size.n],
            re: vec![0.0; size.bins()],
            im: vec![0.0; size.bins()],
        }
    }

    /// Analyse frame `k` of a channel whose first sample sits at absolute index
    /// `offset`. Samples outside the slice read as zero. Result in `self.re/im`.
    pub fn analyse(&mut self, samples: &[f32], offset: i64, k: i64) {
        let start = self.size.frame_start(k) - offset;
        let len = samples.len() as i64;
        for m in 0..self.size.n {
            let i = start + m as i64;
            let x = if i >= 0 && i < len {
                samples[i as usize] as f64
            } else {
                0.0
            };
            self.frame[m] = x * self.window[m];
        }
        self.fft.forward(&self.frame, &mut self.re, &mut self.im);
    }

    /// Power spectrum (|X|²) of the last analysed frame, added into `acc`.
    pub fn accumulate_power(&self, acc: &mut [f64]) {
        for b in 0..self.size.bins() {
            acc[b] += self.re[b] * self.re[b] + self.im[b] * self.im[b];
        }
    }

    /// Inverse-transform `(re, im)` (the caller has scaled them), window, and
    /// overlap-add into `out`, whose first element is absolute sample `out_offset`.
    /// Samples falling outside `out` are dropped.
    pub fn synthesise_add(
        &mut self,
        re: &[f64],
        im: &[f64],
        k: i64,
        out: &mut [f64],
        out_offset: i64,
    ) {
        self.fft.inverse(re, im, &mut self.frame);
        let start = self.size.frame_start(k) - out_offset;
        let len = out.len() as i64;
        let norm = 1.0 / WOLA_GAIN;
        for m in 0..self.size.n {
            let i = start + m as i64;
            if i >= 0 && i < len {
                out[i as usize] += self.frame[m] * self.window[m] * norm;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{amp_to_db, sin};

    #[test]
    fn frame_arithmetic() {
        let s = StftSize::new(16); // hop 4
        assert_eq!(s.frames_overlapping(0, 1), (-1, 2));
        assert_eq!(s.frames_centred_in(0, 8), (0, 1));
        assert_eq!(s.frames_centred_in(1, 9), (1, 2));
        assert_eq!(div_floor(-5, 4), -2);
        assert_eq!(div_ceil(-5, 4), -1);
        assert_eq!(StftSize::scaled(2048, 48000).n, 2048);
        assert_eq!(StftSize::scaled(2048, 44100).n, 2048);
        assert_eq!(StftSize::scaled(2048, 16000).n, 512);
        assert_eq!(StftSize::scaled(2048, 96000).n, 4096);
    }

    /// Analysis followed by synthesis with no modification reconstructs the
    /// signal to well below −120 dB.
    #[test]
    fn round_trip_below_minus_120_db() {
        let len = 10_000;
        let x: Vec<f32> = (0..len)
            .map(|i| (0.5 * sin(i as f64 * 0.013) + 0.3 * sin(i as f64 * 0.71)) as f32)
            .collect();
        let size = StftSize::new(1024);
        let mut stft = Stft::new(size);
        let mut out = vec![0.0f64; len];
        let (k0, k1) = size.frames_overlapping(0, len as i64);
        for k in k0..=k1 {
            stft.analyse(&x, 0, k);
            let (re, im) = (stft.re.clone(), stft.im.clone());
            stft.synthesise_add(&re, &im, k, &mut out, 0);
        }
        let err = x
            .iter()
            .zip(&out)
            .map(|(a, b)| (*a as f64 - b).abs())
            .fold(0.0, f64::max);
        assert!(
            amp_to_db(err) < -120.0,
            "reconstruction error {} dB",
            amp_to_db(err)
        );
    }
}
