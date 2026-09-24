//! Analysis. `Features` v1 is the snapshot recorded before and after every
//! step: the "state" an AI learns from, and the basis for the authenticity
//! tools. Values are rounded to fixed precision so snapshots are compact and
//! stable in JSON.

pub mod levels;
pub mod loudness;
pub mod peaks;
pub mod quiet;
pub mod spectrogram;
pub mod spectrum;
pub mod tonal;

use serde::{Deserialize, Serialize};

use crate::audio::AudioBuffer;
use crate::dsp::stft::StftSize;
use crate::math::{amp_to_db, round_to};
pub use quiet::QuietRegion;
pub use spectrum::BandStat;
pub use tonal::{LineConfig, TonalLine};

/// Version 2: tonal lines must also stand out in the speech pauses, and carry
/// their prominence there (`TonalLine::pause_prominence_db`).
pub const FEATURES_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Hum {
    /// 50 or 60.
    pub fundamental_hz: f64,
    /// Harmonics 1–6 found as lines.
    pub harmonics_found: u32,
    pub strength_db: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Features {
    pub version: u32,
    pub t0: f64,
    pub t1: f64,
    pub sample_rate: u32,
    pub channels: u32,
    pub peak_dbfs: f64,
    pub true_peak_dbfs: f64,
    pub rms_dbfs: f64,
    /// Integrated loudness (BS.1770); absent for silence or spans under 400 ms.
    pub loudness_lufs: Option<f64>,
    pub crest_db: f64,
    /// Mean per channel (linear, full scale = 1).
    pub dc_offset: Vec<f64>,
    pub clipped_samples: u64,
    /// 10th-percentile level of 20 ms frames, dBFS.
    pub noise_floor_dbfs: f64,
    pub octave_bands: Vec<BandStat>,
    pub spectral_centroid_hz: f64,
    pub spectral_tilt_db_per_octave: f64,
    pub tonal_lines: Vec<TonalLine>,
    pub quietest_region: Option<QuietRegion>,
    pub hum: Option<Hum>,
    pub speech_activity_ratio: f64,
    pub bandwidth_hz: f64,
    pub bandwidth_limited: bool,
}

/// Features of `audio` over `[t0, t1)` seconds (the whole clip if `None`).
pub fn features(audio: &AudioBuffer, span: Option<(f64, f64)>) -> Features {
    let (a, b) = match span {
        Some((t0, t1)) => (audio.time_to_sample(t0), audio.time_to_sample(t1)),
        None => (0, audio.len()),
    };
    let b = b.max((a + 1).min(audio.len()));
    let sr = audio.sample_rate as f64;

    let lv = levels::levels(audio, a, b);
    let loud = loudness::integrated_lufs(audio, a, b);
    let lt = spectrum::long_term(audio, a, b, StftSize::scaled(2048, audio.sample_rate));
    let bands = spectrum::octave_bands(&lt);
    let (energies, _, _) = quiet::frame_energies(audio, a, b);
    let noise_floor = if energies.is_empty() {
        crate::math::DB_FLOOR
    } else {
        let mut v: Vec<f32> = energies.iter().map(|&d| d as f32).collect();
        crate::dsp::smooth::quantile_in_place(&mut v, 0.1) as f64
    };
    let (bw, limited) = spectrum::bandwidth_hz(&lt.ltas, lt.bin_hz, audio.sample_rate);

    // One line analysis serves both the line list and hum detection.
    let spec = tonal::LineSpectrum::compute(audio, a, b);
    let lines = tonal::detect_in(&spec, audio, &LineConfig::default());
    let hum_cands = tonal::detect_in(
        &spec,
        audio,
        &LineConfig {
            f_lo: 40.0,
            f_hi: 400.0,
            min_prominence_db: 3.0,
            max_lines: 32,
            ..LineConfig::default()
        },
    );

    Features {
        version: FEATURES_VERSION,
        t0: round_to(a as f64 / sr, 3),
        t1: round_to(b as f64 / sr, 3),
        sample_rate: audio.sample_rate,
        channels: audio.num_channels() as u32,
        peak_dbfs: round_to(amp_to_db(lv.peak), 2),
        true_peak_dbfs: round_to(amp_to_db(lv.true_peak), 2),
        rms_dbfs: round_to(amp_to_db(lv.rms), 2),
        loudness_lufs: loud.map(|l| round_to(l, 2)),
        crest_db: round_to(amp_to_db(lv.peak) - amp_to_db(lv.rms), 2),
        dc_offset: lv.dc_offset.iter().map(|&d| round_to(d, 7)).collect(),
        clipped_samples: lv.clipped_samples,
        noise_floor_dbfs: round_to(noise_floor, 2),
        spectral_centroid_hz: round_to(spectrum::centroid_hz(&lt.ltas, lt.bin_hz), 1),
        spectral_tilt_db_per_octave: round_to(spectrum::tilt_db_per_octave(&bands), 2),
        octave_bands: bands,
        tonal_lines: lines,
        quietest_region: quiet::quietest_region(audio, a, b),
        hum: detect_hum(&hum_cands),
        speech_activity_ratio: quiet::activity_ratio(&energies),
        bandwidth_hz: bw,
        bandwidth_limited: limited,
    }
}

fn detect_hum(lines: &[TonalLine]) -> Option<Hum> {
    let mut best: Option<Hum> = None;
    for base in [50.0, 60.0] {
        let mut found = 0u32;
        let mut strength: f64 = 0.0;
        for h in 1..=6 {
            let f = base * h as f64;
            if let Some(l) = lines.iter().find(|l| (l.freq_hz - f).abs() <= 1.5) {
                found += 1;
                strength = strength.max(l.prominence_db);
            }
        }
        if found >= 2 || (found == 1 && lines.iter().any(|l| (l.freq_hz - base).abs() <= 1.5)) {
            let cand = Hum {
                fundamental_hz: base,
                harmonics_found: found,
                strength_db: strength,
            };
            if best.as_ref().map_or(true, |b| found > b.harmonics_found) {
                best = Some(cand);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{sin, PI};

    #[test]
    fn features_of_hum_and_noise() {
        let sr = 48000u32;
        let mut s = 3u64;
        let x: Vec<f32> = (0..sr as usize * 4)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let n = ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * 0.001;
                let t = i as f64 / sr as f64;
                (n + 0.003 * sin(2.0 * PI * 50.0 * t) + 0.002 * sin(2.0 * PI * 150.0 * t) + 0.002)
                    as f32
            })
            .collect();
        let a = AudioBuffer::mono(sr, x);
        let f = features(&a, None);
        assert_eq!(f.version, FEATURES_VERSION);
        assert!((f.dc_offset[0] - 0.002).abs() < 1e-4);
        let hum = f.hum.clone().expect("hum");
        assert_eq!(hum.fundamental_hz, 50.0);
        assert!(hum.harmonics_found >= 2);
        assert!(f.quietest_region.is_some());
        let json = serde_json::to_value(&f).unwrap();
        assert!(crate::provenance::jcs::canonicalize(&json).is_ok());
    }
}
