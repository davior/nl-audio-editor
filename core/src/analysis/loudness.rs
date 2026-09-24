//! Integrated loudness, ITU-R BS.1770-4 (K-weighting, 400 ms blocks with 75 %
//! overlap, absolute gate −70 LUFS, relative gate −10 LU).

use crate::audio::AudioBuffer;
use crate::dsp::biquad::Biquad;
use crate::math::log10;

pub fn integrated_lufs(audio: &AudioBuffer, a: usize, b: usize) -> Option<f64> {
    let sr = audio.sample_rate as f64;
    let block = (0.4 * sr).round() as usize;
    let step = (0.1 * sr).round() as usize;
    let len = b - a;
    if len < block || step == 0 {
        return None;
    }
    // K-weighted squares per channel.
    let weighted: Vec<Vec<f64>> = audio
        .channels
        .iter()
        .map(|c| {
            let mut s1 = Biquad::k_shelf(sr);
            let mut s2 = Biquad::k_highpass(sr);
            c[a..b]
                .iter()
                .map(|&x| {
                    let y = s2.process(s1.process(x as f64));
                    y * y
                })
                .collect()
        })
        .collect();
    let mut blocks = Vec::new();
    let mut start = 0;
    while start + block <= len {
        let z: f64 = weighted
            .iter()
            .map(|w| w[start..start + block].iter().sum::<f64>() / block as f64)
            .sum();
        blocks.push(z);
        start += step;
    }
    let loud = |z: f64| -0.691 + 10.0 * log10(z);
    let abs_gated: Vec<f64> = blocks
        .iter()
        .copied()
        .filter(|&z| z > 0.0 && loud(z) > -70.0)
        .collect();
    if abs_gated.is_empty() {
        return None;
    }
    let mean_abs = abs_gated.iter().sum::<f64>() / abs_gated.len() as f64;
    let rel_gate = loud(mean_abs) - 10.0;
    let rel: Vec<f64> = abs_gated
        .iter()
        .copied()
        .filter(|&z| loud(z) > rel_gate)
        .collect();
    if rel.is_empty() {
        return None;
    }
    let mean = rel.iter().sum::<f64>() / rel.len() as f64;
    Some(loud(mean))
}

/// Gain in dB that would bring integrated loudness to `target_lufs`.
pub fn gain_to_target(measured: f64, target_lufs: f64) -> f64 {
    target_lufs - measured
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{pow, sin, PI};

    fn amp(db: f64) -> f64 {
        pow(10.0, db / 20.0)
    }

    /// A 1 kHz sine with −20 dBFS peaks has a mean square of −23.01 dB; K-weighting adds
    /// ≈ +0.69 dB at 1 kHz and the −0.691 constant takes it back, so it reads ≈ −23 LUFS.
    #[test]
    fn sine_at_minus_20() {
        let sr = 48000u32;
        let a = amp(-20.0);
        let x: Vec<f32> = (0..sr * 3)
            .map(|i| (a * sin(2.0 * PI * 1000.0 * i as f64 / sr as f64)) as f32)
            .collect();
        let buf = AudioBuffer::mono(sr, x);
        let l = integrated_lufs(&buf, 0, buf.len()).unwrap();
        assert!((l - (-23.0)).abs() < 0.2, "{l}");
    }

    #[test]
    fn silence_has_no_loudness() {
        let buf = AudioBuffer::silent(48000, 1, 48000);
        assert!(integrated_lufs(&buf, 0, buf.len()).is_none());
    }
}
