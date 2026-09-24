use crate::math::{cos, PI};

/// Periodic Hann window, `0.5 − 0.5·cos(2πm/n)`.
pub fn hann_periodic(n: usize) -> Vec<f64> {
    (0..n)
        .map(|m| 0.5 - 0.5 * cos(2.0 * PI * m as f64 / n as f64))
        .collect()
}

/// Square root of the periodic Hann window. Used for both analysis and
/// synthesis, so their product is Hann, which sums to exactly 2 at 75% overlap.
pub fn sqrt_hann_periodic(n: usize) -> Vec<f64> {
    hann_periodic(n).into_iter().map(f64::sqrt).collect()
}

/// Raised-cosine ramp from 0 to 1 over `len` steps: element `i` of `len`.
pub fn ramp(i: usize, len: usize) -> f64 {
    if len == 0 {
        return 1.0;
    }
    let x = (i as f64 + 0.5) / len as f64;
    0.5 - 0.5 * cos(PI * x.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_sums_to_two_at_quarter_hop() {
        let n = 64;
        let w = hann_periodic(n);
        for s in 0..n / 4 {
            let sum: f64 = (0..4).map(|j| w[s + j * n / 4]).sum();
            assert!((sum - 2.0).abs() < 1e-12);
        }
    }
}
