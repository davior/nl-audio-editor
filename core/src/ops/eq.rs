//! Zero-phase EQ on the STFT-mask engine: `high_pass`, `low_pass`, `bell`,
//! `shelf` and `tilt`. Each is a static gain per frequency, the same in every
//! frame of its scope, so there is no phase shift or timing smear and the
//! residual is exactly what changed. The two filters only cut and use the
//! engine's cut-only path; the others may boost.
//!
//! The curves, with `x` the distance in octaves from the operation's
//! frequency:
//! - high-pass and low-pass: the magnitude of a Butterworth filter of
//!   `slope / 6` orders, 3 dB down at the cutoff, never deeper than
//!   `max_depth_db`;
//! - bell: a raised cosine in octaves, `gain_db` at the centre, half of it at
//!   `x = ±width / 2` and nothing beyond `x = ±width`;
//! - shelf: a raised-cosine step over `transition_octaves`, centred on the
//!   corner;
//! - tilt: `db_per_octave × x` around the pivot, capped at `±max_gain_db`.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::mask::{self, Geometry, MaskStats, Reductions};
use super::{copy_window, typed, with, Descriptor, Op, OpError, RenderOut};
use crate::analysis::tonal::LineSpectrum;
use crate::audio::AudioBuffer;
use crate::dsp::stft::StftSize;
use crate::math::{cos, log10, log2, pow, power_to_db, round_to, sin, PI};
use crate::scope::Scope;

/// Frequencies below this are evaluated here, so the DC bin gets a finite value.
const F_MIN: f64 = 1.0;

/// The analysis size for a curve whose finest feature spans `span_hz`: at
/// least ten bins across it, from 2048 to 32768 points at 48 kHz (scaled with
/// the rate).
pub fn eq_size(span_hz: f64, sr: u32) -> StftSize {
    let min = StftSize::scaled(2048, sr).n;
    let max = StftSize::scaled(32768, sr).n;
    let want = (10.0 * sr as f64 / span_hz.max(1e-3)).ceil();
    let n = if want >= max as f64 {
        max
    } else {
        (want as usize).next_power_of_two()
    };
    StftSize::new(n.clamp(min, max))
}

fn butterworth_db(ratio: f64, orders: f64) -> f64 {
    -10.0 * log10(1.0 + pow(ratio, 2.0 * orders))
}

/// High-pass curve (dB, ≤ 0).
pub fn high_pass_db(f: f64, cutoff: f64, slope: f64, depth: f64) -> f64 {
    butterworth_db(cutoff / f.max(F_MIN), slope / 6.0).max(-depth)
}

/// Low-pass curve (dB, ≤ 0).
pub fn low_pass_db(f: f64, cutoff: f64, slope: f64, depth: f64) -> f64 {
    butterworth_db(f.max(F_MIN) / cutoff, slope / 6.0).max(-depth)
}

/// Bell curve (dB).
pub fn bell_db(f: f64, centre: f64, gain: f64, width: f64) -> f64 {
    let x = log2(f.max(F_MIN) / centre);
    if x.abs() >= width {
        0.0
    } else {
        gain * (0.5 + 0.5 * cos(PI * x / width))
    }
}

/// Shelf curve (dB): `gain` below the corner for a low shelf, above it for a high one.
pub fn shelf_db(f: f64, corner: f64, gain: f64, transition: f64, low: bool) -> f64 {
    let x = log2(f.max(F_MIN) / corner);
    let h = transition / 2.0;
    let below = if x <= -h {
        1.0
    } else if x >= h {
        0.0
    } else {
        0.5 - 0.5 * sin(PI * x / transition)
    };
    gain * if low { below } else { 1.0 - below }
}

/// Tilt curve (dB).
pub fn tilt_db(f: f64, pivot: f64, per_octave: f64, cap: f64) -> f64 {
    (per_octave * log2(f.max(F_MIN) / pivot)).clamp(-cap, cap)
}

/// A curve sampled at each analysis bin.
fn curve(size: StftSize, sr: u32, g: impl Fn(f64) -> f64) -> Vec<f32> {
    let bin_hz = size.bin_hz(sr);
    (0..size.bins())
        .map(|b| g(b as f64 * bin_hz) as f32)
        .collect()
}

/// Apply a static curve; `signed` curves may boost, the others can only cut.
fn apply(
    gains: &[f32],
    signed: bool,
    size: StftSize,
    scope: &Scope,
    input: &AudioBuffer,
    offset: i64,
    a: i64,
    b: i64,
    clip_len: usize,
) -> (AudioBuffer, MaskStats) {
    let geom = Geometry::new(scope, size, input.sample_rate, clip_len, false);
    let r = if signed {
        Reductions::Gains(gains)
    } else {
        Reductions::Static(gains)
    };
    mask::render(&geom, &r, true, input, None, offset, a, b)
}

/// Level of a band from the long-term spectrum of a buffer (dB).
fn band_level_db(buf: &AudioBuffer, lo: f64, hi: f64) -> f64 {
    let sp = LineSpectrum::compute(buf, 0, buf.len());
    let bin = sp.bin_hz();
    let last = sp.ltas.len() - 1;
    let b0 = ((lo.max(0.0) / bin).ceil() as usize).min(last);
    let b1 = ((hi / bin).floor() as usize).min(last).max(b0);
    power_to_db(sp.ltas[b0..=b1].iter().sum())
}

/// Spectral centroid of a buffer's long-term spectrum (Hz).
fn centroid_hz(buf: &AudioBuffer) -> f64 {
    let sp = LineSpectrum::compute(buf, 0, buf.len());
    let bin = sp.bin_hz();
    let (num, den) = sp
        .ltas
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(n, d), (i, &p)| {
            (n + i as f64 * bin * p, d + p)
        });
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

fn below_nyquist(what: &str, f: f64, sr: u32) -> Result<(), OpError> {
    let nyq = sr as f64 / 2.0;
    if f >= nyq {
        return Err(OpError::Invalid(format!(
            "the {what} ({f} Hz) is not below half the sample rate ({nyq} Hz)"
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
struct Sized {
    fft_n: usize,
}

fn radius_of(resolved: &Value) -> usize {
    typed::<Sized>(resolved)
        .map(|r| mask::radius(StftSize::new(r.fft_n), 0))
        .unwrap_or(0)
}

/// Everything an EQ needs to render: its curve and whether it may boost.
trait Curve {
    fn gains(&self, resolved: &Value, size: StftSize, sr: u32) -> Result<Vec<f32>, OpError>;
    fn signed(&self) -> bool;
    fn measure(
        &self,
        resolved: &Value,
        before: &AudioBuffer,
        after: &AudioBuffer,
        stats: &MaskStats,
    ) -> Result<Map<String, Value>, OpError>;
}

fn render_curve(
    c: &dyn Curve,
    resolved: &Value,
    scope: &Scope,
    input: &AudioBuffer,
    offset: i64,
    a: i64,
    b: i64,
    clip_len: usize,
) -> Result<RenderOut, OpError> {
    let size = StftSize::new(typed::<Sized>(resolved)?.fft_n);
    let g = c.gains(resolved, size, input.sample_rate)?;
    let (audio, stats) = apply(&g, c.signed(), size, scope, input, offset, a, b, clip_len);
    let before = copy_window(input, offset, a, b);
    let measurements = c.measure(resolved, &before, &audio, &stats)?;
    Ok(RenderOut {
        audio,
        measurements,
    })
}

fn render_curve_linear(
    c: &dyn Curve,
    resolved: &Value,
    scope: &Scope,
    target: &AudioBuffer,
) -> Result<Option<AudioBuffer>, OpError> {
    // A static curve decides nothing from the signal.
    let size = StftSize::new(typed::<Sized>(resolved)?.fft_n);
    let g = c.gains(resolved, size, target.sample_rate)?;
    let len = target.len();
    let (audio, _) = apply(&g, c.signed(), size, scope, target, 0, 0, len as i64, len);
    Ok(Some(audio))
}

/// The `Op` methods every EQ shares: render through its curve.
macro_rules! curve_op {
    ($t:ty) => {
        impl Op for $t {
            fn descriptor(&self) -> &Descriptor {
                &self.desc
            }

            fn resolve(
                &self,
                params: &Value,
                _scope: &Scope,
                input: &AudioBuffer,
            ) -> Result<Value, OpError> {
                self.resolve_curve(params, input.sample_rate)
            }

            fn radius(&self, resolved: &Value, _sr: u32) -> usize {
                radius_of(resolved)
            }

            fn render_linear(
                &self,
                resolved: &Value,
                scope: &Scope,
                _decide: &AudioBuffer,
                target: &AudioBuffer,
            ) -> Result<Option<AudioBuffer>, OpError> {
                render_curve_linear(self, resolved, scope, target)
            }

            fn render(
                &self,
                resolved: &Value,
                scope: &Scope,
                input: &AudioBuffer,
                offset: i64,
                a: i64,
                b: i64,
                clip_len: usize,
            ) -> Result<RenderOut, OpError> {
                render_curve(self, resolved, scope, input, offset, a, b, clip_len)
            }
        }
    };
}

// ---------------------------------------------------------------- high_pass, low_pass

/// `high_pass` (`high: true`) and `low_pass`.
pub struct Pass {
    pub desc: Descriptor,
    pub high: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassParams {
    cutoff_hz: f64,
    slope_db_per_octave: f64,
    max_depth_db: f64,
}

impl Pass {
    fn resolve_curve(&self, params: &Value, sr: u32) -> Result<Value, OpError> {
        let p: PassParams = typed(params)?;
        below_nyquist("cutoff", p.cutoff_hz, sr)?;
        let size = eq_size(p.cutoff_hz / 2.0, sr);
        Ok(with(params, json!({ "fft_n": size.n })))
    }
}

impl Curve for Pass {
    fn gains(&self, resolved: &Value, size: StftSize, sr: u32) -> Result<Vec<f32>, OpError> {
        let p: PassParams = typed(&strip(resolved))?;
        Ok(curve(size, sr, |f| {
            if self.high {
                high_pass_db(f, p.cutoff_hz, p.slope_db_per_octave, p.max_depth_db)
            } else {
                low_pass_db(f, p.cutoff_hz, p.slope_db_per_octave, p.max_depth_db)
            }
        }))
    }

    fn signed(&self) -> bool {
        false
    }

    fn measure(
        &self,
        resolved: &Value,
        before: &AudioBuffer,
        after: &AudioBuffer,
        stats: &MaskStats,
    ) -> Result<Map<String, Value>, OpError> {
        let p: PassParams = typed(&strip(resolved))?;
        let nyq = before.sample_rate as f64 / 2.0;
        let (lo, hi, name) = if self.high {
            (0.0, p.cutoff_hz, "below")
        } else {
            (p.cutoff_hz, nyq, "above")
        };
        let mut m = Map::new();
        m.insert(
            "energy_removed_db".into(),
            json!(round_to(stats.energy_removed_db(), 2)),
        );
        m.insert(
            format!("level_{name}_before_db"),
            json!(round_to(band_level_db(before, lo, hi), 2)),
        );
        m.insert(
            format!("level_{name}_after_db"),
            json!(round_to(band_level_db(after, lo, hi), 2)),
        );
        Ok(m)
    }
}

curve_op!(Pass);

/// The parameters of a resolved EQ step, without what resolving added.
fn strip(resolved: &Value) -> Value {
    let mut m = resolved.as_object().cloned().unwrap_or_default();
    m.remove("fft_n");
    Value::Object(m)
}

// ---------------------------------------------------------------- bell

pub struct Bell {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BellParams {
    freq_hz: f64,
    gain_db: f64,
    width_octaves: f64,
}

impl Bell {
    fn resolve_curve(&self, params: &Value, sr: u32) -> Result<Value, OpError> {
        let p: BellParams = typed(params)?;
        below_nyquist("centre", p.freq_hz, sr)?;
        // Finest feature: from the half-gain point below the centre to the centre.
        let span = p.freq_hz * (1.0 - pow(2.0, -p.width_octaves / 2.0));
        Ok(with(params, json!({ "fft_n": eq_size(span, sr).n })))
    }
}

impl Curve for Bell {
    fn gains(&self, resolved: &Value, size: StftSize, sr: u32) -> Result<Vec<f32>, OpError> {
        let p: BellParams = typed(&strip(resolved))?;
        Ok(curve(size, sr, |f| {
            bell_db(f, p.freq_hz, p.gain_db, p.width_octaves)
        }))
    }

    fn signed(&self) -> bool {
        true
    }

    fn measure(
        &self,
        resolved: &Value,
        before: &AudioBuffer,
        after: &AudioBuffer,
        stats: &MaskStats,
    ) -> Result<Map<String, Value>, OpError> {
        let p: BellParams = typed(&strip(resolved))?;
        let (lo, hi) = (
            p.freq_hz * pow(2.0, -p.width_octaves / 2.0),
            p.freq_hz * pow(2.0, p.width_octaves / 2.0),
        );
        let mut m = Map::new();
        m.insert(
            "level_change_db".into(),
            json!(round_to(stats.level_change_db(), 2)),
        );
        m.insert(
            "band_level_before_db".into(),
            json!(round_to(band_level_db(before, lo, hi), 2)),
        );
        m.insert(
            "band_level_after_db".into(),
            json!(round_to(band_level_db(after, lo, hi), 2)),
        );
        Ok(m)
    }
}

curve_op!(Bell);

// ---------------------------------------------------------------- shelf

pub struct Shelf {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShelfParams {
    kind: String,
    freq_hz: f64,
    gain_db: f64,
    transition_octaves: f64,
}

impl Shelf {
    fn resolve_curve(&self, params: &Value, sr: u32) -> Result<Value, OpError> {
        let p: ShelfParams = typed(params)?;
        below_nyquist("corner", p.freq_hz, sr)?;
        let span = p.freq_hz * (1.0 - pow(2.0, -p.transition_octaves / 2.0));
        Ok(with(params, json!({ "fft_n": eq_size(span, sr).n })))
    }
}

impl Curve for Shelf {
    fn gains(&self, resolved: &Value, size: StftSize, sr: u32) -> Result<Vec<f32>, OpError> {
        let p: ShelfParams = typed(&strip(resolved))?;
        let low = p.kind == "low";
        Ok(curve(size, sr, |f| {
            shelf_db(f, p.freq_hz, p.gain_db, p.transition_octaves, low)
        }))
    }

    fn signed(&self) -> bool {
        true
    }

    fn measure(
        &self,
        resolved: &Value,
        before: &AudioBuffer,
        after: &AudioBuffer,
        stats: &MaskStats,
    ) -> Result<Map<String, Value>, OpError> {
        let p: ShelfParams = typed(&strip(resolved))?;
        let nyq = before.sample_rate as f64 / 2.0;
        let (lo, hi) = if p.kind == "low" {
            (0.0, p.freq_hz)
        } else {
            (p.freq_hz, nyq)
        };
        let mut m = Map::new();
        m.insert(
            "level_change_db".into(),
            json!(round_to(stats.level_change_db(), 2)),
        );
        m.insert(
            "shelf_level_before_db".into(),
            json!(round_to(band_level_db(before, lo, hi), 2)),
        );
        m.insert(
            "shelf_level_after_db".into(),
            json!(round_to(band_level_db(after, lo, hi), 2)),
        );
        Ok(m)
    }
}

curve_op!(Shelf);

// ---------------------------------------------------------------- tilt

pub struct Tilt {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TiltParams {
    db_per_octave: f64,
    pivot_hz: f64,
    max_gain_db: f64,
}

impl Tilt {
    fn resolve_curve(&self, params: &Value, sr: u32) -> Result<Value, OpError> {
        let p: TiltParams = typed(params)?;
        below_nyquist("pivot", p.pivot_hz, sr)?;
        // Finest feature: the octave below the lower corner, where the cap begins.
        let corner = if p.db_per_octave == 0.0 {
            p.pivot_hz
        } else {
            (p.pivot_hz * pow(2.0, -p.max_gain_db / p.db_per_octave.abs())).max(20.0)
        };
        Ok(with(
            params,
            json!({ "fft_n": eq_size(corner / 2.0, sr).n }),
        ))
    }
}

impl Curve for Tilt {
    fn gains(&self, resolved: &Value, size: StftSize, sr: u32) -> Result<Vec<f32>, OpError> {
        let p: TiltParams = typed(&strip(resolved))?;
        Ok(curve(size, sr, |f| {
            tilt_db(f, p.pivot_hz, p.db_per_octave, p.max_gain_db)
        }))
    }

    fn signed(&self) -> bool {
        true
    }

    fn measure(
        &self,
        _resolved: &Value,
        before: &AudioBuffer,
        after: &AudioBuffer,
        stats: &MaskStats,
    ) -> Result<Map<String, Value>, OpError> {
        let mut m = Map::new();
        m.insert(
            "level_change_db".into(),
            json!(round_to(stats.level_change_db(), 2)),
        );
        m.insert(
            "centroid_before_hz".into(),
            json!(round_to(centroid_hz(before), 1)),
        );
        m.insert(
            "centroid_after_hz".into(),
            json!(round_to(centroid_hz(after), 1)),
        );
        Ok(m)
    }
}

curve_op!(Tilt);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::amp_to_db;

    #[test]
    fn curves_have_their_stated_shapes() {
        // Butterworth: 3 dB down at the cutoff, the slope per octave well below it, capped.
        assert!((high_pass_db(80.0, 80.0, 24.0, 60.0) + 3.01).abs() < 0.01);
        assert!((high_pass_db(20.0, 80.0, 24.0, 60.0) + 48.16).abs() < 0.01);
        assert_eq!(high_pass_db(1.0, 80.0, 24.0, 60.0), -60.0);
        assert!(high_pass_db(1000.0, 80.0, 24.0, 60.0) > -1e-6);
        assert!((low_pass_db(8000.0, 8000.0, 12.0, 60.0) + 3.01).abs() < 0.01);
        assert_eq!(high_pass_db(20.0, 80.0, 24.0, 0.0), 0.0);
        // Bell: full gain at the centre, half at half a width, none a width away.
        assert_eq!(bell_db(1000.0, 1000.0, 6.0, 1.0), 6.0);
        assert!((bell_db(1000.0 * pow(2.0, 0.5), 1000.0, 6.0, 1.0) - 3.0).abs() < 1e-9);
        assert_eq!(bell_db(2000.0, 1000.0, 6.0, 1.0), 0.0);
        // Shelf: the gain on its side, half at the corner, none on the other side.
        assert_eq!(shelf_db(50.0, 200.0, -4.0, 1.0, true), -4.0);
        assert!((shelf_db(200.0, 200.0, -4.0, 1.0, true) + 2.0).abs() < 1e-9);
        assert_eq!(shelf_db(800.0, 200.0, -4.0, 1.0, true), 0.0);
        assert_eq!(shelf_db(800.0, 200.0, 5.0, 1.0, false), 5.0);
        // Tilt: nothing at the pivot, so much per octave, capped.
        assert_eq!(tilt_db(1000.0, 1000.0, 2.0, 12.0), 0.0);
        assert!((tilt_db(4000.0, 1000.0, 2.0, 12.0) - 4.0).abs() < 1e-9);
        assert_eq!(tilt_db(1.0, 1000.0, 2.0, 12.0), -12.0);
    }

    #[test]
    fn eq_sizes_resolve_the_finest_feature() {
        let n = |span: f64| eq_size(span, 48_000).n;
        assert_eq!(n(40.0), 16384); // an 80 Hz high-pass: the octave below the cutoff
        assert_eq!(n(4000.0), 2048);
        assert_eq!(n(1.0), 32768);
        assert_eq!(eq_size(40.0, 96_000).n, 32768);
    }

    fn sine(freq: f64, sr: u32, secs: f64) -> AudioBuffer {
        let len = (sr as f64 * secs) as usize;
        AudioBuffer::mono(
            sr,
            (0..len)
                .map(|i| (0.25 * sin(2.0 * PI * freq * i as f64 / sr as f64)) as f32)
                .collect(),
        )
    }

    /// Gain (dB) a resolved EQ applies to a sine, measured over the middle of the clip.
    fn gain_at(op: &dyn Op, params: Value, freq: f64) -> f64 {
        let sr = 48_000;
        let input = sine(freq, sr, 4.0);
        let params = op.descriptor().normalise_params(&params).unwrap();
        let resolved = op.resolve(&params, &Scope::Clip, &input).unwrap();
        let len = input.len();
        let out = op
            .render(&resolved, &Scope::Clip, &input, 0, 0, len as i64, len)
            .unwrap()
            .audio;
        let rms = |b: &AudioBuffer| {
            let s = &b.channels[0][len / 4..3 * len / 4];
            (s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / s.len() as f64).sqrt()
        };
        amp_to_db(rms(&out) / rms(&input))
    }

    fn op(id: &str) -> &'static dyn Op {
        crate::ops::registry().latest(id).unwrap()
    }

    #[test]
    fn rendered_gains_follow_the_curves() {
        let hp = json!({"cutoff_hz": 80, "slope_db_per_octave": 24});
        assert!(gain_at(op("high_pass"), hp.clone(), 1000.0).abs() < 0.05);
        assert!((gain_at(op("high_pass"), hp.clone(), 80.0) + 3.0).abs() < 0.5);
        assert!((gain_at(op("high_pass"), hp, 40.0) + 24.0).abs() < 1.0);
        let lp = json!({"cutoff_hz": 4000, "slope_db_per_octave": 24});
        assert!(gain_at(op("low_pass"), lp.clone(), 500.0).abs() < 0.05);
        assert!((gain_at(op("low_pass"), lp, 8000.0) + 24.0).abs() < 1.0);
        let bell = json!({"freq_hz": 1000, "gain_db": 6});
        assert!((gain_at(op("bell"), bell.clone(), 1000.0) - 6.0).abs() < 0.3);
        assert!(gain_at(op("bell"), bell, 4000.0).abs() < 0.1);
        let shelf = json!({"kind": "high", "freq_hz": 2000, "gain_db": -6});
        assert!((gain_at(op("shelf"), shelf.clone(), 8000.0) + 6.0).abs() < 0.3);
        assert!(gain_at(op("shelf"), shelf, 250.0).abs() < 0.1);
        let tilt = json!({"db_per_octave": 2});
        assert!((gain_at(op("tilt"), tilt.clone(), 4000.0) - 4.0).abs() < 0.3);
        assert!((gain_at(op("tilt"), tilt, 250.0) + 4.0).abs() < 0.3);
        // A boost adds energy; its identity setting leaves the sine untouched.
        let flat = json!({"freq_hz": 1000, "gain_db": 0});
        assert_eq!(gain_at(op("bell"), flat, 1000.0), 0.0);
    }

    #[test]
    fn a_frequency_at_or_above_nyquist_is_refused() {
        let input = sine(1000.0, 16_000, 0.5);
        let params = op("low_pass")
            .descriptor()
            .normalise_params(&json!({"cutoff_hz": 8000}))
            .unwrap();
        let e = op("low_pass")
            .resolve(&params, &Scope::Clip, &input)
            .unwrap_err();
        assert!(e.to_string().contains("half the sample rate"), "{e}");
    }
}
