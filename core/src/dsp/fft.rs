//! A small, auditable FFT in f64.
//!
//! Written here rather than taken from a crate so that the arithmetic is
//! identical on every target: radix-2, fixed operation order, twiddles from
//! `libm`, no SIMD and no fused multiply-add. Real transforms of length `n` run
//! as a complex transform of length `n/2` plus a split step.

use crate::math::{cos, sin, PI};

/// Complex FFT of power-of-two length, iterative radix-2 decimation in time.
#[derive(Clone, Debug)]
pub struct ComplexFft {
    n: usize,
    /// `cos(2πk/n)` and `sin(2πk/n)` for `k < n/2`.
    tw_cos: Vec<f64>,
    tw_sin: Vec<f64>,
    bitrev: Vec<usize>,
}

impl ComplexFft {
    pub fn new(n: usize) -> Self {
        assert!(
            n >= 1 && n.is_power_of_two(),
            "FFT length must be a power of two"
        );
        let bits = n.trailing_zeros();
        let bitrev = (0..n)
            .map(|i| {
                if bits == 0 {
                    0
                } else {
                    i.reverse_bits() >> (usize::BITS - bits)
                }
            })
            .collect();
        let half = n / 2;
        let tw_cos = (0..half)
            .map(|k| cos(2.0 * PI * k as f64 / n as f64))
            .collect();
        let tw_sin = (0..half)
            .map(|k| sin(2.0 * PI * k as f64 / n as f64))
            .collect();
        ComplexFft {
            n,
            tw_cos,
            tw_sin,
            bitrev,
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// In-place transform. `inverse` uses the conjugate twiddles and does not scale.
    pub fn process(&self, re: &mut [f64], im: &mut [f64], inverse: bool) {
        let n = self.n;
        assert!(re.len() == n && im.len() == n);
        for i in 0..n {
            let j = self.bitrev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let sign = if inverse { 1.0 } else { -1.0 };
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            let mut start = 0;
            while start < n {
                for j in 0..half {
                    let wr = self.tw_cos[j * step];
                    let wi = sign * self.tw_sin[j * step];
                    let a = start + j;
                    let b = a + half;
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
                start += len;
            }
            len <<= 1;
        }
    }
}

/// Real FFT of power-of-two length `n ≥ 4`. The spectrum has `n/2 + 1` bins.
#[derive(Clone, Debug)]
pub struct RealFft {
    n: usize,
    m: usize,
    cfft: ComplexFft,
    /// `W_n^k = e^{-2πik/n}` for `k < n/2`, split into real and imaginary parts.
    w_re: Vec<f64>,
    w_im: Vec<f64>,
    zr: Vec<f64>,
    zi: Vec<f64>,
}

impl RealFft {
    pub fn new(n: usize) -> Self {
        assert!(
            n >= 4 && n.is_power_of_two(),
            "real FFT length must be a power of two ≥ 4"
        );
        let m = n / 2;
        let w_re = (0..m)
            .map(|k| cos(2.0 * PI * k as f64 / n as f64))
            .collect();
        let w_im = (0..m)
            .map(|k| -sin(2.0 * PI * k as f64 / n as f64))
            .collect();
        RealFft {
            n,
            m,
            cfft: ComplexFft::new(m),
            w_re,
            w_im,
            zr: vec![0.0; m],
            zi: vec![0.0; m],
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn bins(&self) -> usize {
        self.m + 1
    }

    /// Unnormalised forward transform. `re`/`im` must hold `n/2 + 1` bins.
    pub fn forward(&mut self, input: &[f64], re: &mut [f64], im: &mut [f64]) {
        let (n, m) = (self.n, self.m);
        assert!(input.len() == n && re.len() == m + 1 && im.len() == m + 1);
        for k in 0..m {
            self.zr[k] = input[2 * k];
            self.zi[k] = input[2 * k + 1];
        }
        self.cfft.process(&mut self.zr, &mut self.zi, false);
        // X[0] and X[m] are real.
        re[0] = self.zr[0] + self.zi[0];
        im[0] = 0.0;
        re[m] = self.zr[0] - self.zi[0];
        im[m] = 0.0;
        for k in 1..m {
            let (ar, ai) = (self.zr[k], self.zi[k]);
            let (br, bi) = (self.zr[m - k], -self.zi[m - k]); // conj(Z[m-k])
                                                              // Even part E = (Z[k] + conj(Z[m-k])) / 2
            let er = 0.5 * (ar + br);
            let ei = 0.5 * (ai + bi);
            // Odd part O = (Z[k] - conj(Z[m-k])) / 2i
            let dr = ar - br;
            let di = ai - bi;
            let or = 0.5 * di;
            let oi = -0.5 * dr;
            // X[k] = E + W^k O
            let (wr, wi) = (self.w_re[k], self.w_im[k]);
            re[k] = er + (wr * or - wi * oi);
            im[k] = ei + (wr * oi + wi * or);
        }
    }

    /// Inverse transform, normalised so that `inverse(forward(x)) == x` up to rounding.
    /// The imaginary parts of bins `0` and `n/2` are ignored.
    pub fn inverse(&mut self, re: &[f64], im: &[f64], output: &mut [f64]) {
        let (n, m) = (self.n, self.m);
        assert!(output.len() == n && re.len() == m + 1 && im.len() == m + 1);
        for k in 0..m {
            let (xr, xi) = (re[k], if k == 0 { 0.0 } else { im[k] });
            let j = m - k;
            let (cr, ci) = (re[j], if j == m { 0.0 } else { -im[j] }); // conj(X[m-k])
            let er = 0.5 * (xr + cr);
            let ei = 0.5 * (xi + ci);
            // O = (X[k] - conj(X[m-k])) * W^{-k} / 2
            let dr = 0.5 * (xr - cr);
            let di = 0.5 * (xi - ci);
            let (wr, wi) = (self.w_re[k], -self.w_im[k]);
            let or = dr * wr - di * wi;
            let oi = dr * wi + di * wr;
            // Z = E + i·O
            self.zr[k] = er - oi;
            self.zi[k] = ei + or;
        }
        self.cfft.process(&mut self.zr, &mut self.zi, true);
        let scale = 1.0 / m as f64;
        for k in 0..m {
            output[2 * k] = self.zr[k] * scale;
            output[2 * k + 1] = self.zi[k] * scale;
        }
        let _ = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_dft(x: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let n = x.len();
        let mut re = vec![0.0; n / 2 + 1];
        let mut im = vec![0.0; n / 2 + 1];
        for k in 0..=n / 2 {
            for (t, &v) in x.iter().enumerate() {
                let a = -2.0 * PI * (k * t) as f64 / n as f64;
                re[k] += v * cos(a);
                im[k] += v * sin(a);
            }
        }
        (re, im)
    }

    fn signal(n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| {
                sin(0.37 * i as f64) + 0.25 * cos(1.9 * i as f64 + 0.3) + (i % 7) as f64 * 0.01
            })
            .collect()
    }

    #[test]
    fn matches_naive_dft() {
        for &n in &[4usize, 8, 16, 64, 256] {
            let x = signal(n);
            let mut fft = RealFft::new(n);
            let mut re = vec![0.0; n / 2 + 1];
            let mut im = vec![0.0; n / 2 + 1];
            fft.forward(&x, &mut re, &mut im);
            let (nr, ni) = naive_dft(&x);
            for k in 0..=n / 2 {
                assert!(
                    (re[k] - nr[k]).abs() < 1e-9,
                    "n={n} k={k} re {} vs {}",
                    re[k],
                    nr[k]
                );
                assert!(
                    (im[k] - ni[k]).abs() < 1e-9,
                    "n={n} k={k} im {} vs {}",
                    im[k],
                    ni[k]
                );
            }
        }
    }

    #[test]
    fn inverse_round_trips() {
        for &n in &[4usize, 32, 2048, 16384] {
            let x = signal(n);
            let mut fft = RealFft::new(n);
            let mut re = vec![0.0; n / 2 + 1];
            let mut im = vec![0.0; n / 2 + 1];
            fft.forward(&x, &mut re, &mut im);
            let mut y = vec![0.0; n];
            fft.inverse(&re, &im, &mut y);
            let err = x
                .iter()
                .zip(&y)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            assert!(err < 1e-12, "n={n} err={err}");
        }
    }

    #[test]
    fn is_deterministic() {
        let x = signal(1024);
        let run = || {
            let mut fft = RealFft::new(1024);
            let mut re = vec![0.0; 513];
            let mut im = vec![0.0; 513];
            fft.forward(&x, &mut re, &mut im);
            re.iter()
                .chain(im.iter())
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
