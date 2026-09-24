//! Deterministic golden clips built from separate, known components.
//!
//! Because every component is known, a test can apply the exact processing a
//! step used (the mask or envelope decided on the mixture) to each component
//! separately and measure what happened to the voices, the noise, each line,
//! the whine and the bangs. Clip A is shaped like the reference material
//! (`docs/spec/reference-workflows/audacity-spoken-word-cleanup.md`): very
//! quiet, DC offset, steady lines at 750 Hz and 3,150 Hz. Clip B has the same
//! kinds of problem elsewhere.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use crate::audio::AudioBuffer;
use crate::dsp::biquad::Biquad;
use crate::math::{cos, db_to_amp, exp, ln, sin, PI};

/// xorshift64* — small, fast, identical on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    /// Standard normal (Box–Muller).
    pub fn gauss(&mut self) -> f64 {
        let u1 = self.uniform().max(1e-300);
        let u2 = self.uniform();
        (-2.0 * ln(u1)).sqrt() * cos(2.0 * PI * u2)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VoiceSpec {
    pub seed: u64,
    pub f0_lo: f64,
    pub f0_hi: f64,
    pub formant_scale: f64,
    /// Peak level relative to the foreground voice (dB).
    pub level_db: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Spec {
    pub name: String,
    pub seed: u64,
    pub sample_rate: u32,
    pub duration_s: f64,
    pub foreground: VoiceSpec,
    pub background: VoiceSpec,
    /// Silence for both voices (the natural place for a noise profile).
    pub long_pause: (f64, f64),
    /// Steady lines: (frequency, amplitude relative to the foreground peak).
    pub lines: Vec<(f64, f64)>,
    pub hum_hz: f64,
    pub hum_amplitudes: Vec<f64>,
    pub whine_hz: f64,
    pub whine_amplitude: f64,
    pub whine_spans: Vec<(f64, f64)>,
    pub bang_times: Vec<f64>,
    pub bang_amplitude: f64,
    pub noise_rms: f64,
    pub rumble_rms: f64,
    /// Final peak of the mix before the DC offset is added.
    pub peak_dbfs: f64,
    pub dc: f64,
    pub clip_span: (f64, f64),
}

pub fn spec_a() -> Spec {
    Spec {
        name: "golden_a".into(),
        seed: 0xA,
        sample_rate: 48000,
        duration_s: 60.0,
        foreground: VoiceSpec {
            seed: 101,
            f0_lo: 105.0,
            f0_hi: 150.0,
            formant_scale: 1.0,
            level_db: 0.0,
        },
        background: VoiceSpec {
            seed: 202,
            f0_lo: 185.0,
            f0_hi: 240.0,
            formant_scale: 1.17,
            level_db: -24.0,
        },
        long_pause: (40.0, 42.5),
        lines: vec![(750.0, 0.035), (3150.0, 0.0032)],
        hum_hz: 50.0,
        hum_amplitudes: vec![0.02, 0.012, 0.008],
        whine_hz: 5200.0,
        whine_amplitude: 0.03,
        whine_spans: vec![(10.0, 14.0), (30.0, 33.0), (47.0, 50.0)],
        bang_times: vec![7.3, 21.8, 38.2, 52.6],
        bang_amplitude: 1.4,
        noise_rms: 0.0079,
        rumble_rms: 0.004,
        peak_dbfs: -30.0,
        dc: 0.0015,
        clip_span: (44.0, 44.6),
    }
}

pub fn spec_b() -> Spec {
    Spec {
        name: "golden_b".into(),
        seed: 0xB,
        sample_rate: 48000,
        duration_s: 60.0,
        foreground: VoiceSpec {
            seed: 303,
            f0_lo: 150.0,
            f0_hi: 195.0,
            formant_scale: 1.1,
            level_db: 0.0,
        },
        background: VoiceSpec {
            seed: 404,
            f0_lo: 95.0,
            f0_hi: 130.0,
            formant_scale: 0.95,
            level_db: -20.0,
        },
        long_pause: (18.0, 20.5),
        lines: vec![(620.0, 0.03), (2450.0, 0.006)],
        hum_hz: 60.0,
        hum_amplitudes: vec![0.018, 0.01, 0.006],
        whine_hz: 6100.0,
        whine_amplitude: 0.03,
        whine_spans: vec![(5.0, 8.0), (26.0, 29.5), (51.0, 54.0)],
        bang_times: vec![12.4, 33.7, 45.1],
        bang_amplitude: 1.3,
        noise_rms: 0.009,
        rumble_rms: 0.003,
        peak_dbfs: -26.0,
        dc: -0.001,
        clip_span: (36.0, 36.5),
    }
}

/// A generated clip: the mixture and each component. The mixture equals the
/// sum of the components to within float rounding (the flat tops of the
/// clipped span are exact in the mixture).
pub struct Clip {
    pub spec: Spec,
    pub mix: AudioBuffer,
    pub components: BTreeMap<String, AudioBuffer>,
    /// Sample spans where each voice is speaking.
    pub voice_spans: BTreeMap<String, Vec<(f64, f64)>>,
}

impl Clip {
    pub fn component(&self, name: &str) -> &AudioBuffer {
        &self.components[name]
    }

    pub fn meta(&self) -> Value {
        json!({ "spec": self.spec, "voice_spans": self.voice_spans })
    }
}

const VOWELS: [[f64; 3]; 5] = [
    [730.0, 1090.0, 2440.0], // a
    [530.0, 1840.0, 2480.0], // e
    [270.0, 2290.0, 3010.0], // i
    [570.0, 840.0, 2410.0],  // o
    [300.0, 870.0, 2240.0],  // u
];

/// Speech-like voice: glottal pulses shaped by three formants, in syllables
/// and phrases, with pitch that keeps moving (so harmonics never form lines).
fn voice(
    v: &VoiceSpec,
    sr: u32,
    len: usize,
    pause: (f64, f64),
    duration: f64,
) -> (Vec<f64>, Vec<(f64, f64)>) {
    let mut rng = Rng::new(v.seed);
    let srf = sr as f64;
    let mut out = vec![0.0f64; len];
    let mut spans = Vec::new();
    let mut t = rng.range(0.5, 1.5);
    while t < duration - 1.0 {
        let phrase = rng.range(1.5, 4.0);
        let end = (t + phrase).min(duration - 0.5);
        // No speech across the long pause.
        if t < pause.1 && end > pause.0 {
            t = pause.1 + rng.range(0.1, 0.4);
            continue;
        }
        spans.push((t, end));
        let f_start = rng.range(v.f0_hi * 0.9, v.f0_hi);
        let f_end = rng.range(v.f0_lo, v.f0_lo * 1.1);
        let mut st = t;
        while st < end - 0.1 {
            let dur = rng.range(0.12, 0.3).min(end - st);
            let vowel = VOWELS[(rng.uniform() * VOWELS.len() as f64) as usize % VOWELS.len()];
            let gain = rng.range(0.6, 1.0);
            let fricative = rng.uniform() < 0.2;
            let wobble = rng.range(-0.08, 0.08);
            let s0 = (st * srf) as usize;
            let s1 = ((st + dur) * srf) as usize;
            let mut formants: Vec<Biquad> = vowel
                .iter()
                .enumerate()
                .map(|(i, &f)| {
                    Biquad::bandpass(
                        srf,
                        f * v.formant_scale,
                        f * v.formant_scale / (80.0 + 20.0 * i as f64),
                    )
                })
                .collect();
            let fgain = [1.0, 0.5, 0.25];
            let mut glottal = Biquad::lowpass(srf, 700.0, 0.7);
            let mut phase = 0.0;
            for i in s0..s1.min(len) {
                let tt = i as f64 / srf;
                let prog = (tt - t) / (end - t);
                let f0 = (f_start + (f_end - f_start) * prog)
                    * (1.0 + wobble * sin(2.0 * PI * 3.0 * (tt - st)));
                phase += f0 / srf;
                let pulse = if phase >= 1.0 {
                    phase -= 1.0;
                    1.0
                } else {
                    0.0
                };
                let src = glottal.process(pulse) + 0.02 * rng.gauss();
                let mut y = 0.0;
                for (fi, f) in formants.iter_mut().enumerate() {
                    y += fgain[fi] * f.process(src);
                }
                let pos = (i - s0) as f64 / srf;
                let rem = (s1 - i) as f64 / srf;
                let env = (pos / 0.02).min(1.0).min(rem / 0.04);
                out[i] += y * gain * env;
            }
            if fricative {
                let f0s = s0.saturating_sub((0.06 * srf) as usize);
                let mut bp = Biquad::bandpass(srf, 5500.0, 1.2);
                for i in f0s..s0.min(len) {
                    let env = sin(PI * (i - f0s) as f64 / (s0 - f0s).max(1) as f64);
                    out[i] += 0.25 * gain * env * bp.process(rng.gauss()) * 0.05;
                }
            }
            st += dur + rng.range(0.03, 0.08);
        }
        t = end + rng.range(0.4, 1.5);
    }
    let peak = out.iter().fold(0.0f64, |m, &x| m.max(x.abs()));
    let g = db_to_amp(v.level_db) / peak.max(1e-12);
    out.iter_mut().for_each(|x| *x *= g);
    (out, spans)
}

pub fn generate(spec: &Spec) -> Clip {
    let sr = spec.sample_rate;
    let srf = sr as f64;
    let len = (spec.duration_s * srf) as usize;
    let mut rng = Rng::new(spec.seed);
    let mut comps: BTreeMap<String, Vec<f64>> = BTreeMap::new();

    let (fg, fg_spans) = voice(&spec.foreground, sr, len, spec.long_pause, spec.duration_s);
    let (bg, bg_spans) = voice(&spec.background, sr, len, spec.long_pause, spec.duration_s);
    comps.insert("foreground".into(), fg);
    comps.insert("background".into(), bg);

    comps.insert(
        "noise".into(),
        (0..len).map(|_| spec.noise_rms * rng.gauss()).collect(),
    );
    let mut lp1 = Biquad::lowpass(srf, 45.0, 0.7);
    let mut lp2 = Biquad::lowpass(srf, 45.0, 0.7);
    let mut rumble: Vec<f64> = (0..len)
        .map(|_| lp2.process(lp1.process(rng.gauss())))
        .collect();
    let rrms = (rumble.iter().map(|x| x * x).sum::<f64>() / len as f64).sqrt();
    rumble
        .iter_mut()
        .for_each(|x| *x *= spec.rumble_rms / rrms.max(1e-12));
    comps.insert("rumble".into(), rumble);

    for (i, &(f, a)) in spec.lines.iter().enumerate() {
        let ph = rng.range(0.0, 2.0 * PI);
        comps.insert(
            format!("line_{}", i + 1),
            (0..len)
                .map(|n| a * sin(2.0 * PI * f * n as f64 / srf + ph))
                .collect(),
        );
    }
    let hum: Vec<f64> = (0..len)
        .map(|n| {
            spec.hum_amplitudes
                .iter()
                .enumerate()
                .map(|(h, &a)| a * sin(2.0 * PI * spec.hum_hz * (h + 1) as f64 * n as f64 / srf))
                .sum()
        })
        .collect();
    comps.insert("hum".into(), hum);

    let fade = 0.05;
    comps.insert(
        "whine".into(),
        (0..len)
            .map(|n| {
                let t = n as f64 / srf;
                let env = spec
                    .whine_spans
                    .iter()
                    .map(|&(a, b)| {
                        if t >= a && t < b {
                            ((t - a) / fade).min(1.0).min((b - t) / fade)
                        } else {
                            0.0
                        }
                    })
                    .fold(0.0, f64::max);
                if env > 0.0 {
                    spec.whine_amplitude * env * sin(2.0 * PI * spec.whine_hz * t)
                } else {
                    0.0
                }
            })
            .collect(),
    );

    let mut bangs = vec![0.0f64; len];
    for &bt in &spec.bang_times {
        let s0 = (bt * srf) as usize;
        let n = (0.06 * srf) as usize;
        let mut burst: Vec<f64> = (0..n)
            .map(|i| rng.gauss() * exp(-(i as f64) / (0.015 * srf)))
            .collect();
        let peak = burst.iter().fold(0.0f64, |m, &x| m.max(x.abs()));
        burst
            .iter_mut()
            .for_each(|x| *x *= spec.bang_amplitude / peak.max(1e-12));
        for (i, v) in burst.iter().enumerate() {
            if s0 + i < len {
                bangs[s0 + i] += v;
            }
        }
    }
    comps.insert("bangs".into(), bangs);

    // Scale everything so the mixture peaks at the target, then round each
    // component to f32 once; the mixture is the exact sum of the rounded parts.
    let mix_f64: Vec<f64> = (0..len)
        .map(|i| comps.values().map(|c| c[i]).sum())
        .collect();
    let peak = mix_f64.iter().fold(0.0f64, |m, &x| m.max(x.abs()));
    let scale = db_to_amp(spec.peak_dbfs) / peak;
    let mut components: BTreeMap<String, Vec<f32>> = comps
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().map(|x| (x * scale) as f32).collect()))
        .collect();
    components.insert("dc".into(), vec![spec.dc as f32; len]);
    let mut mix: Vec<f32> = (0..len)
        .map(|i| components.values().map(|c| c[i] as f64).sum::<f64>() as f32)
        .collect();

    // Clipping of the recording in one span: flat tops at 35 % of the span's
    // peak, written directly into the mixture so they are exactly flat. The
    // "clipping" component is what the flattening changed.
    let (c0, c1) = (
        (spec.clip_span.0 * srf) as usize,
        (spec.clip_span.1 * srf) as usize,
    );
    let dc = spec.dc as f32;
    let span_peak = mix[c0..c1]
        .iter()
        .fold(0.0f32, |m, &x| m.max((x - dc).abs()));
    let limit = 0.35 * span_peak;
    let mut clipping = vec![0.0f32; len];
    for i in c0..c1 {
        let centred = mix[i] - dc;
        if centred.abs() > limit {
            let flat = limit.copysign(centred) + dc;
            clipping[i] = (flat as f64 - mix[i] as f64) as f32;
            mix[i] = flat;
        }
    }
    components.insert("clipping".into(), clipping);
    let mut voice_spans = BTreeMap::new();
    voice_spans.insert("foreground".to_string(), fg_spans);
    voice_spans.insert("background".to_string(), bg_spans);
    Clip {
        spec: spec.clone(),
        mix: AudioBuffer::mono(sr, mix),
        components: components
            .into_iter()
            .map(|(k, v)| (k, AudioBuffer::mono(sr, v)))
            .collect(),
        voice_spans,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_and_the_mix_is_the_sum_of_its_parts() {
        let mut s = spec_a();
        s.duration_s = 6.0;
        s.long_pause = (3.0, 4.0);
        s.whine_spans = vec![(1.0, 2.0)];
        s.bang_times = vec![2.5];
        s.clip_span = (5.0, 5.2);
        let a = generate(&s);
        let b = generate(&s);
        assert_eq!(a.mix.render_hash(), b.mix.render_hash());
        for i in (0..a.mix.len()).step_by(97) {
            let sum: f64 = a.components.values().map(|c| c.channels[0][i] as f64).sum();
            assert!(
                (sum - a.mix.channels[0][i] as f64).abs() < 1e-8,
                "sample {i}"
            );
        }
        let f = crate::analysis::features(&a.mix, None);
        assert!(
            f.clipped_samples > 20,
            "flat tops are detected: {}",
            f.clipped_samples
        );
        let peak = a.mix.channels[0]
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!((crate::math::amp_to_db(peak as f64) - (-30.0)).abs() < 0.5);
    }
}
