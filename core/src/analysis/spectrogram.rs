//! Spectrogram images for display. Each pixel column max-pools every analysis
//! frame inside its time span, so short events (a click, a bang) stay visible
//! however far the view is zoomed out.

use serde::{Deserialize, Serialize};

use super::spectrum::FramePower;
use crate::audio::AudioBuffer;
use crate::dsp::stft::StftSize;
use crate::math::{log2, pow, power_to_db};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum FreqScale {
    Linear,
    Log,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SpectrogramRequest {
    pub t0: f64,
    pub t1: f64,
    pub columns: u32,
    pub rows: u32,
    pub f_min: f64,
    pub f_max: f64,
    pub scale: FreqScale,
    pub db_min: f64,
    pub db_max: f64,
    /// FFT size at 48 kHz (scaled with the sample rate). Typically 2048.
    pub fft_at_48k: u32,
}

/// Returns `rows × columns` levels (0–255), row 0 at the top (highest frequency).
pub fn spectrogram(audio: &AudioBuffer, req: &SpectrogramRequest) -> Vec<u8> {
    spectrogram_columns(audio, req, 0, req.columns as usize)
}

/// Columns `[c0, c1)` of a request: `rows × (c1 − c0)` levels, exactly the
/// same values as those columns of the whole image (each column is computed
/// on its own), so a long view can be delivered in parts.
pub fn spectrogram_columns(
    audio: &AudioBuffer,
    req: &SpectrogramRequest,
    c0: usize,
    c1: usize,
) -> Vec<u8> {
    let all = req.columns as usize;
    let c1 = c1.min(all);
    let c0 = c0.min(c1);
    let cols = c1 - c0;
    let rows = req.rows as usize;
    let mut out = vec![0u8; rows * cols];
    if cols == 0 || rows == 0 || req.t1 <= req.t0 {
        return out;
    }
    let sr = audio.sample_rate as f64;
    let size = StftSize::scaled(req.fft_at_48k as usize, audio.sample_rate);
    let mut fp = FramePower::new(size, audio.sample_rate);
    let bins = size.bins();
    let bin_hz = fp.bin_hz();
    let nyq = sr / 2.0;
    let f_max = req.f_max.min(nyq);
    let f_min = match req.scale {
        FreqScale::Log => req.f_min.max(bin_hz),
        FreqScale::Linear => req.f_min.max(0.0),
    };
    // Row r covers [lo, hi) in Hz.
    let row_edges: Vec<(f64, f64)> = (0..rows)
        .map(|r| {
            let (u0, u1) = (
                (rows - 1 - r) as f64 / rows as f64,
                (rows - r) as f64 / rows as f64,
            );
            match req.scale {
                FreqScale::Linear => (f_min + u0 * (f_max - f_min), f_min + u1 * (f_max - f_min)),
                FreqScale::Log => {
                    let (l0, l1) = (log2(f_min), log2(f_max));
                    (pow(2.0, l0 + u0 * (l1 - l0)), pow(2.0, l0 + u1 * (l1 - l0)))
                }
            }
        })
        .collect();
    let col_span = (req.t1 - req.t0) / all as f64;
    let mut col_power = vec![0.0f64; bins];
    let range = (req.db_max - req.db_min).max(1e-6);
    for (i, c) in (c0..c1).enumerate() {
        let a = ((req.t0 + c as f64 * col_span) * sr).floor() as i64;
        let b = (((req.t0 + (c + 1) as f64 * col_span) * sr).floor() as i64).max(a + 1);
        let (mut k0, mut k1) = size.frames_centred_in(a, b);
        if k1 < k0 {
            // Column narrower than a hop: use the frame nearest its centre.
            let mid = (a + b) / 2;
            k0 = (mid as f64 / size.hop as f64).round() as i64;
            k1 = k0;
        }
        col_power.iter_mut().for_each(|v| *v = 0.0);
        for k in k0..=k1 {
            let p = fp.compute(audio, k);
            for (m, &v) in col_power.iter_mut().zip(p) {
                if v > *m {
                    *m = v;
                }
            }
        }
        for (r, &(lo, hi)) in row_edges.iter().enumerate() {
            let b0 = (lo / bin_hz).floor() as usize;
            let b1 = ((hi / bin_hz).ceil() as usize).max(b0 + 1).min(bins);
            let p = col_power[b0.min(bins - 1)..b1]
                .iter()
                .cloned()
                .fold(0.0, f64::max);
            let v = ((power_to_db(p) - req.db_min) / range * 255.0)
                .round()
                .clamp(0.0, 255.0);
            out[r * cols + i] = v as u8;
        }
    }
    out
}

/// Perceptual colour map (inferno), 0–255 → RGB.
pub fn inferno(level: u8) -> [u8; 3] {
    let t = level as f64 / 255.0;
    let c = [
        [
            0.0002189403691192265,
            0.001651004631001012,
            -0.01948089843709184,
        ],
        [0.1065134194856116, 0.5639564367884091, 3.932712388889277],
        [11.60249308247187, -3.972853965665698, -15.9423941062914],
        [-41.70399613139459, 17.43639888205313, 44.35414519872813],
        [77.162935699427, -33.40235894210092, -81.80730925738993],
        [-71.31942824499214, 32.62606426397723, 73.20951985803202],
        [25.13112622477341, -12.24266895238567, -23.07032500287172],
    ];
    let mut rgb = [0u8; 3];
    for (i, v) in rgb.iter_mut().enumerate() {
        let mut acc = c[6][i];
        for k in (0..6).rev() {
            acc = c[k][i] + t * acc;
        }
        *v = (acc.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{sin, PI};

    #[test]
    fn a_sine_lights_the_right_row() {
        let sr = 48000u32;
        let x: Vec<f32> = (0..sr)
            .map(|i| (0.5 * sin(2.0 * PI * 6000.0 * i as f64 / sr as f64)) as f32)
            .collect();
        let a = AudioBuffer::mono(sr, x);
        let req = SpectrogramRequest {
            t0: 0.0,
            t1: 1.0,
            columns: 10,
            rows: 24,
            f_min: 0.0,
            f_max: 24000.0,
            scale: FreqScale::Linear,
            db_min: -120.0,
            db_max: 0.0,
            fft_at_48k: 2048,
        };
        let img = spectrogram(&a, &req);
        // 6 kHz of 24 kHz in 24 rows, top row highest: row 17 covers 6–7 kHz or row 18 covers 5–6 kHz.
        let col = 5;
        let brightest = (0..24).max_by_key(|&r| img[r * 10 + col]).unwrap();
        assert!(brightest == 17 || brightest == 18, "row {brightest}");
        assert!(inferno(0).iter().all(|&v| v < 8));
        assert!(inferno(255)[0] > 240);
    }

    #[test]
    fn a_view_delivered_in_parts_equals_the_whole() {
        let sr = 48000u32;
        let x: Vec<f32> = (0..sr as usize * 3)
            .map(|i| {
                let t = i as f64 / sr as f64;
                (0.1 * sin(2.0 * PI * 1000.0 * t)
                    + if (1.0..1.01).contains(&t) { 0.5 } else { 0.0 }) as f32
            })
            .collect();
        let a = AudioBuffer::mono(sr, x);
        let req = SpectrogramRequest {
            t0: 0.25,
            t1: 2.9,
            columns: 301,
            rows: 64,
            f_min: 0.0,
            f_max: 8000.0,
            scale: FreqScale::Linear,
            db_min: -200.0,
            db_max: 0.0,
            fft_at_48k: 2048,
        };
        let whole = spectrogram(&a, &req);
        let mut parts = vec![0u8; whole.len()];
        for (c0, c1) in [(0, 100), (100, 250), (250, 301)] {
            let p = spectrogram_columns(&a, &req, c0, c1);
            let w = c1 - c0;
            for r in 0..64 {
                parts[r * 301 + c0..r * 301 + c1].copy_from_slice(&p[r * w..(r + 1) * w]);
            }
        }
        assert_eq!(parts, whole);
    }
}
