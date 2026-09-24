//! Second-order IIR sections (RBJ cookbook). Used only for analysis (loudness
//! K-weighting) and for synthesising test material, never on a render path,
//! because an IIR filter's memory is unbounded.

use crate::math::{cos, sin, tan, PI};

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Constant-peak-gain band-pass (peak gain = 1 at `f0`).
    pub fn bandpass(sample_rate: f64, f0: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * f0 / sample_rate;
        let alpha = sin(w0) / (2.0 * q);
        Biquad::new(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos(w0), 1.0 - alpha)
    }

    pub fn lowpass(sample_rate: f64, f0: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * f0 / sample_rate;
        let alpha = sin(w0) / (2.0 * q);
        let c = cos(w0);
        Biquad::new(
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    pub fn highpass(sample_rate: f64, f0: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * f0 / sample_rate;
        let alpha = sin(w0) / (2.0 * q);
        let c = cos(w0);
        Biquad::new(
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    /// ITU-R BS.1770 K-weighting stage 1 (high shelf, ~+4 dB above ~1.5 kHz),
    /// designed for any sample rate by the bilinear transform.
    pub fn k_shelf(sample_rate: f64) -> Self {
        let f0 = 1681.974450955533;
        let g = 3.999843853973347;
        let q = 0.7071752369554196;
        let k = tan(PI * f0 / sample_rate);
        let vh = crate::math::pow(10.0, g / 20.0);
        let vb = crate::math::pow(vh, 0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Biquad::new(
            vh + vb * k / q + k * k,
            2.0 * (k * k - vh),
            vh - vb * k / q + k * k,
            a0,
            2.0 * (k * k - 1.0),
            1.0 - k / q + k * k,
        )
    }

    /// ITU-R BS.1770 K-weighting stage 2 (high-pass, ~38 Hz).
    pub fn k_highpass(sample_rate: f64) -> Self {
        let f0 = 38.13547087602444;
        let q = 0.5003270373238773;
        let k = tan(PI * f0 / sample_rate);
        let a0 = 1.0 + k / q + k * k;
        Biquad::new(1.0, -2.0, 1.0, a0, 2.0 * (k * k - 1.0), 1.0 - k / q + k * k)
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        // Transposed direct form II.
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_weighting_matches_bs1770_coefficients_at_48k() {
        let s = Biquad::k_shelf(48000.0);
        assert!((s.b0 - 1.53512485958697).abs() < 1e-9);
        assert!((s.b1 + 2.69169618940638).abs() < 1e-9);
        assert!((s.b2 - 1.19839281085285).abs() < 1e-9);
        assert!((s.a1 + 1.69065929318241).abs() < 1e-9);
        assert!((s.a2 - 0.73248077421585).abs() < 1e-9);
        let h = Biquad::k_highpass(48000.0);
        assert!((h.a1 + 1.99004745483398).abs() < 1e-9);
        assert!((h.a2 - 0.99007225036621).abs() < 1e-9);
    }
}
