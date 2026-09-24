//! `line_reduce`, `band_cut` and `spectral_compressor`.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::compressor::knee_curve;
use super::mask::{self, Geometry, Levels, Reductions};
use super::{typed, with, Descriptor, Op, OpError, RenderOut};
use crate::analysis::tonal::{self, LineConfig, LineSpectrum};
use crate::audio::AudioBuffer;
use crate::dsp::smooth::{
    attack_release, min_then_mean, quantile_in_place, sliding_median, sliding_quantile,
};
use crate::dsp::stft::StftSize;
use crate::math::{cos, round_to, PI};
use crate::scope::Scope;

/// Raised-cosine shape: 1 inside `[lo + e/2, hi − e/2]`, 0 outside
/// `[lo − e/2, hi + e/2]`.
fn band_shape(f: f64, lo: f64, hi: f64, e: f64) -> f64 {
    let rc = |x: f64| {
        if x <= 0.0 {
            0.0
        } else if x >= 1.0 {
            1.0
        } else {
            0.5 - 0.5 * cos(PI * x)
        }
    };
    if e <= 0.0 {
        return if f >= lo && f <= hi { 1.0 } else { 0.0 };
    }
    rc((f - (lo - e / 2.0)) / e).min(rc(((hi + e / 2.0) - f) / e))
}

// ---------------------------------------------------------------- line_reduce

pub struct LineReduce {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineParams {
    lines: Value,
    min_prominence_db: f64,
    min_persistence: f64,
    max_lines: i64,
    target_excess_db: f64,
    max_depth_db: f64,
    width_factor: f64,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct CutLine {
    pub freq_hz: f64,
    pub width_hz: f64,
    pub depth_db: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prominence_db: Option<f64>,
}

#[derive(Deserialize)]
struct LineResolved {
    width_factor: f64,
    fft_n: usize,
    resolved_lines: Vec<CutLine>,
}

/// Static reduction per bin for a set of lines: full depth over the line's
/// (scaled) width, raised-cosine shoulders of half that width (at least two
/// bins) on each side; overlapping lines take the deeper cut.
pub fn line_mask(lines: &[CutLine], width_factor: f64, size: StftSize, sr: u32) -> Vec<f32> {
    let bins = size.bins();
    let bin_hz = size.bin_hz(sr);
    let mut red = vec![0.0f32; bins];
    for l in lines {
        let core = l.width_hz * width_factor;
        let edge = (core / 2.0).max(2.0 * bin_hz);
        let lo = l.freq_hz - core / 2.0;
        let hi = l.freq_hz + core / 2.0;
        let b0 = (((lo - edge) / bin_hz).floor().max(0.0)) as usize;
        let b1 = (((hi + edge) / bin_hz).ceil() as usize).min(bins - 1);
        for b in b0..=b1 {
            let s = band_shape(b as f64 * bin_hz, lo, hi, edge * 2.0);
            let v = (-l.depth_db * s) as f32;
            if v < red[b] {
                red[b] = v;
            }
        }
    }
    red
}

impl Op for LineReduce {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: LineParams = typed(params)?;
        let sr = input.sample_rate;
        let (s0, s1) = scope.samples(sr, input.len());
        let spec = LineSpectrum::compute(input, s0, s1);
        let size = spec.size;
        let mut lines: Vec<CutLine> = if p.lines.as_str() == Some("auto") {
            let (f_lo, f_hi) = scope.band_or_full(sr);
            let cfg = LineConfig {
                f_lo,
                f_hi,
                min_prominence_db: p.min_prominence_db,
                min_persistence: p.min_persistence,
                max_lines: p.max_lines as usize,
                ..LineConfig::default()
            };
            tonal::detect_in(&spec, input, &cfg)
                .into_iter()
                .map(|l| CutLine {
                    freq_hz: l.freq_hz,
                    width_hz: l.width_hz,
                    depth_db: (l.prominence_db - p.target_excess_db).clamp(0.0, p.max_depth_db),
                    prominence_db: Some(l.prominence_db),
                })
                .collect()
        } else {
            serde_json::from_value(p.lines.clone())
                .map_err(|e| OpError::Invalid(format!("lines: {e}")))?
        };
        // The manual loop — cut, look again, cut further until nothing stands
        // out — evaluated on the long-term spectrum, which a static mask scales
        // exactly.
        let db = spec.db();
        let w = ((LineConfig::default().neighbourhood_hz / spec.bin_hz()).round() as usize).max(4);
        let mut predicted = 0.0f64;
        let auto = p.lines.as_str() == Some("auto");
        for _ in 0..5 {
            let red = line_mask(&lines, p.width_factor, size, sr);
            let after: Vec<f64> = db.iter().zip(&red).map(|(d, r)| d + *r as f64).collect();
            let hood = LineSpectrum::neighbourhood(&after, w);
            let mut changed = false;
            predicted = 0.0;
            for l in lines.iter_mut() {
                let rem = spec.prominence_at(&after, &hood, l.freq_hz).max(0.0);
                predicted = predicted.max(rem);
                if auto && rem > p.target_excess_db + 0.5 && l.depth_db < p.max_depth_db {
                    l.depth_db = (l.depth_db + rem - p.target_excess_db).min(p.max_depth_db);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for l in lines.iter_mut() {
            l.depth_db = round_to(l.depth_db, 2);
        }
        Ok(with(
            params,
            json!({
                "fft_n": size.n,
                "resolved_lines": lines,
                "predicted_max_remaining_prominence_db": round_to(predicted, 2),
            }),
        ))
    }

    fn radius(&self, resolved: &Value, _sr: u32) -> usize {
        typed::<LineResolved>(resolved)
            .map(|r| mask::radius(StftSize::new(r.fft_n), 0))
            .unwrap_or(0)
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        // A static mask decides nothing from the signal.
        let r: LineResolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let red = line_mask(&r.resolved_lines, r.width_factor, size, target.sample_rate);
        let len = target.len();
        let geom = Geometry::new(scope, size, target.sample_rate, len, false);
        let (audio, _) = mask::render(
            &geom,
            &Reductions::Static(&red),
            true,
            target,
            None,
            0,
            0,
            len as i64,
        );
        Ok(Some(audio))
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
        let r: LineResolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let sr = input.sample_rate;
        let red = line_mask(&r.resolved_lines, r.width_factor, size, sr);
        let geom = Geometry::new(scope, size, sr, clip_len, false);
        let (audio, st) = mask::render(
            &geom,
            &Reductions::Static(&red),
            true,
            input,
            None,
            offset,
            a,
            b,
        );
        let mut m = Map::new();
        m.insert(
            "lines_reduced".into(),
            json!(r.resolved_lines.iter().filter(|l| l.depth_db > 0.0).count()),
        );
        m.insert(
            "energy_removed_db".into(),
            json!(round_to(st.energy_removed_db(), 2)),
        );
        // The stop measurement: how far each treated line still stands out.
        let spec = LineSpectrum::compute(&audio, 0, audio.len());
        let db = spec.db();
        let w = ((LineConfig::default().neighbourhood_hz / spec.bin_hz()).round() as usize).max(4);
        let hood = LineSpectrum::neighbourhood(&db, w);
        let remaining = r
            .resolved_lines
            .iter()
            .map(|l| spec.prominence_at(&db, &hood, l.freq_hz).max(0.0))
            .fold(0.0f64, f64::max);
        m.insert(
            "max_remaining_prominence_db".into(),
            json!(round_to(remaining, 2)),
        );
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}

// ---------------------------------------------------------------- band_cut

pub struct BandCut {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // parsed to check the parameter set against the descriptor
struct BandParams {
    f_lo: f64,
    f_hi: f64,
    depth_db: f64,
    edge_hz: Value,
}

#[derive(Deserialize)]
struct BandResolved {
    f_lo: f64,
    f_hi: f64,
    depth_db: f64,
    fft_n: usize,
    edge_hz_resolved: f64,
}

fn band_size(f_lo: f64, f_hi: f64, sr: u32) -> StftSize {
    let min = StftSize::scaled(1024, sr).n;
    let max = StftSize::scaled(16384, sr).n;
    let want = (4.0 * sr as f64 / (f_hi - f_lo)).ceil() as usize;
    StftSize::new(want.next_power_of_two().clamp(min, max))
}

impl Op for BandCut {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        _scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: BandParams = typed(params)?;
        let nyq = input.sample_rate as f64 / 2.0;
        if p.f_hi <= p.f_lo || p.f_lo >= nyq {
            return Err(OpError::Invalid(format!(
                "band {}–{} Hz is empty or above the Nyquist frequency ({nyq} Hz)",
                p.f_lo, p.f_hi
            )));
        }
        let size = band_size(p.f_lo, p.f_hi.min(nyq), input.sample_rate);
        let edge = p
            .edge_hz
            .as_f64()
            .unwrap_or(2.0 * size.bin_hz(input.sample_rate));
        Ok(with(
            params,
            json!({ "fft_n": size.n, "edge_hz_resolved": edge }),
        ))
    }

    fn radius(&self, resolved: &Value, _sr: u32) -> usize {
        typed::<BandResolved>(resolved)
            .map(|r| mask::radius(StftSize::new(r.fft_n), 0))
            .unwrap_or(0)
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        _decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: BandResolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let bin_hz = size.bin_hz(target.sample_rate);
        let red: Vec<f32> = (0..size.bins())
            .map(|bb| {
                (-r.depth_db * band_shape(bb as f64 * bin_hz, r.f_lo, r.f_hi, r.edge_hz_resolved))
                    as f32
            })
            .collect();
        let len = target.len();
        let geom = Geometry::new(scope, size, target.sample_rate, len, false);
        let (audio, _) = mask::render(
            &geom,
            &Reductions::Static(&red),
            true,
            target,
            None,
            0,
            0,
            len as i64,
        );
        Ok(Some(audio))
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
        let r: BandResolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let sr = input.sample_rate;
        let bin_hz = size.bin_hz(sr);
        let red: Vec<f32> = (0..size.bins())
            .map(|bb| {
                (-r.depth_db * band_shape(bb as f64 * bin_hz, r.f_lo, r.f_hi, r.edge_hz_resolved))
                    as f32
            })
            .collect();
        let geom = Geometry::new(scope, size, sr, clip_len, false);
        let (audio, st) = mask::render(
            &geom,
            &Reductions::Static(&red),
            true,
            input,
            None,
            offset,
            a,
            b,
        );
        // Band level before/after from the long-term spectrum of the window.
        let level = |buf: &AudioBuffer| {
            let sp = LineSpectrum::compute(buf, 0, buf.len());
            let (lo, hi) = (
                (r.f_lo / sp.bin_hz()).ceil() as usize,
                (r.f_hi / sp.bin_hz()).floor() as usize,
            );
            let hi = hi.min(sp.ltas.len() - 1);
            crate::math::power_to_db(sp.ltas[lo.min(hi)..=hi].iter().sum())
        };
        let before = super::copy_window(input, offset, a, b);
        let mut m = Map::new();
        m.insert(
            "energy_removed_db".into(),
            json!(round_to(st.energy_removed_db(), 2)),
        );
        m.insert(
            "band_level_before_db".into(),
            json!(round_to(level(&before), 2)),
        );
        m.insert(
            "band_level_after_db".into(),
            json!(round_to(level(&audio), 2)),
        );
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}

// ---------------------------------------------------------------- spectral_compressor

pub struct SpectralCompressor {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // parsed to check the parameter set; resolved values are read via ScResolved
struct ScParams {
    mode: String,
    threshold_db: Value,
    ratio: f64,
    knee_db: f64,
    max_reduction_db: f64,
    attack_ms: f64,
    release_ms: f64,
    resolution: String,
    tonal_width_hz: f64,
    transient_width_ms: f64,
    level_context_s: f64,
    link_channels: bool,
}

#[derive(Deserialize)]
struct ScResolved {
    mode: String,
    threshold_db: f64,
    ratio: f64,
    knee_db: f64,
    max_reduction_db: f64,
    attack_ms: f64,
    release_ms: f64,
    tonal_width_hz: f64,
    transient_width_ms: f64,
    level_context_s: f64,
    link_channels: bool,
    fft_n: usize,
}

struct ScShape {
    wf: usize,
    wt: usize,
    wl: usize,
    att: usize,
    rel: usize,
}

impl ScResolved {
    fn shape(&self, size: StftSize, sr: u32) -> ScShape {
        let hop_s = size.hop as f64 / sr as f64;
        let bin_hz = size.bin_hz(sr);
        let frames = |s: f64| (s / hop_s).round() as usize;
        ScShape {
            wf: ((self.tonal_width_hz / bin_hz).round() as usize).max(2),
            wt: if matches!(self.mode.as_str(), "transient" | "auto") {
                frames(self.transient_width_ms / 2000.0).max(2)
            } else {
                0
            },
            wl: if self.mode == "level" {
                frames(self.level_context_s).max(2)
            } else {
                0
            },
            att: frames(self.attack_ms / 1000.0),
            rel: frames(self.release_ms / 1000.0),
        }
    }
}

/// Reference quantile over neighbouring moments in `transient` mode. A brief
/// burst fills well under a quarter of its neighbourhood; sustained sound such
/// as speech fills most of it, so it is not mistaken for a transient.
pub const TRANSIENT_QUANTILE: f64 = 0.75;

/// Threshold used when `threshold_db` is `auto`: in `level` mode the reference
/// is the band's background, and 20 dB above it leaves a quieter voice mostly
/// untouched; elsewhere 6 dB above the neighbourhood.
pub fn auto_threshold_db(mode: &str) -> f64 {
    if mode == "level" {
        20.0
    } else {
        6.0
    }
}

impl SpectralCompressor {
    fn compute<'a>(
        r: &'a ScResolved,
        geom: &Geometry,
        sr: u32,
    ) -> (usize, impl Fn(&Levels, i64, i64) -> Vec<f32> + 'a) {
        let sh = r.shape(geom.size, sr);
        let context = sh.att + sh.rel + sh.wt.max(sh.wl);
        let (b_lo, b_hi) = (geom.b_lo, geom.b_hi);
        let compute = move |lv: &Levels, k0: i64, k1: i64| -> Vec<f32> {
            let bins = lv.bins;
            let out_frames = (k1 - k0 + 1) as usize;
            let mut out = vec![0.0f32; out_frames * bins];
            if r.ratio == 1.0 || r.max_reduction_db == 0.0 {
                return out;
            }
            // Raw reductions for frames lo..=hi (enough for the envelope).
            let lo = k0 - sh.rel as i64;
            let hi = k1 + sh.att as i64;
            let span = (hi - lo + 1) as usize;
            let nb = b_hi - b_lo + 1;
            // Reference level per cell.
            let mut reference = vec![0.0f32; span * nb];
            let frames_all: Vec<i64> = (lv.k0..lv.k0 + lv.frames as i64).collect();
            let level_of = |k: i64, b: usize| lv.at(k, b);
            match r.mode.as_str() {
                "level" => {
                    // The band's background: the median cell per frame (robust to a
                    // few loud partials or a steady line), then the median of that
                    // over ±wl frames. The loud layer is what rises far above it.
                    let per_frame: Vec<f32> = frames_all
                        .iter()
                        .map(|&k| {
                            let mut v: Vec<f32> = (b_lo..=b_hi).map(|b| level_of(k, b)).collect();
                            quantile_in_place(&mut v, 0.5)
                        })
                        .collect();
                    let background = sliding_median(&per_frame, sh.wl);
                    for (si, k) in (lo..=hi).enumerate() {
                        let m = background[(k - lv.k0) as usize];
                        for bi in 0..nb {
                            reference[si * nb + bi] = m;
                        }
                    }
                }
                mode => {
                    let tonal = matches!(mode, "tonal" | "auto");
                    let transient = matches!(mode, "transient" | "auto");
                    let mut rf = vec![f32::INFINITY; span * nb];
                    let mut rt = vec![f32::INFINITY; span * nb];
                    if tonal {
                        for (si, k) in (lo..=hi).enumerate() {
                            let row: Vec<f32> = (0..bins).map(|b| level_of(k, b)).collect();
                            let med = sliding_median(&row, sh.wf);
                            for bi in 0..nb {
                                rf[si * nb + bi] = med[b_lo + bi];
                            }
                        }
                    }
                    if transient {
                        for bi in 0..nb {
                            let col: Vec<f32> =
                                frames_all.iter().map(|&k| level_of(k, b_lo + bi)).collect();
                            let med = sliding_quantile(&col, sh.wt, TRANSIENT_QUANTILE);
                            for (si, k) in (lo..=hi).enumerate() {
                                rt[si * nb + bi] = med[(k - lv.k0) as usize];
                            }
                        }
                    }
                    for i in 0..reference.len() {
                        reference[i] = rf[i].min(rt[i]);
                    }
                }
            }
            // Gain computer, then envelope over time per bin, then frequency smoothing.
            let mut env = vec![vec![0.0f32; span]; nb];
            for bi in 0..nb {
                let raw: Vec<f32> = (lo..=hi)
                    .enumerate()
                    .map(|(si, k)| {
                        let over = level_of(k, b_lo + bi) as f64
                            - reference[si * nb + bi] as f64
                            - r.threshold_db;
                        knee_curve(over, r.ratio, r.knee_db).max(-r.max_reduction_db) as f32
                    })
                    .collect();
                env[bi] = attack_release(&raw, sh.att, sh.rel);
            }
            for fi in 0..out_frames {
                let si = fi + sh.rel;
                let row: Vec<f32> = (0..nb).map(|bi| env[bi][si]).collect();
                let smoothed = min_then_mean(&row, 1);
                for bi in 0..nb {
                    out[fi * bins + b_lo + bi] = smoothed[bi];
                }
            }
            out
        };
        (context, compute)
    }
}

fn resolution_n(resolution: &str, mode: &str) -> usize {
    match (resolution, mode) {
        ("fine_time", _) | ("auto", "transient") => 1024,
        ("fine_frequency", _) | ("auto", "tonal") => 4096,
        _ => 2048,
    }
}

impl Op for SpectralCompressor {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        _scope: &Scope,
        input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: ScParams = typed(params)?;
        let n = StftSize::scaled(resolution_n(&p.resolution, &p.mode), input.sample_rate).n;
        let threshold = p
            .threshold_db
            .as_f64()
            .unwrap_or_else(|| auto_threshold_db(&p.mode));
        Ok(with(
            params,
            json!({ "fft_n": n, "threshold_db": threshold }),
        ))
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        let Ok(r) = typed::<ScResolved>(resolved) else {
            return 0;
        };
        let size = StftSize::new(r.fft_n);
        let sh = r.shape(size, sr);
        mask::radius(size, sh.att + sh.rel + sh.wt.max(sh.wl))
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: ScResolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let sr = target.sample_rate;
        let len = target.len();
        let geom = Geometry::new(scope, size, sr, len, true);
        let (context, compute) = Self::compute(&r, &geom, sr);
        let reductions = Reductions::Dynamic {
            context,
            compute: &compute,
        };
        let (audio, _) = mask::render(
            &geom,
            &reductions,
            r.link_channels,
            target,
            Some(decide),
            0,
            0,
            len as i64,
        );
        Ok(Some(audio))
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
        let r: ScResolved = typed(resolved)?;
        let size = StftSize::new(r.fft_n);
        let sr = input.sample_rate;
        let geom = Geometry::new(scope, size, sr, clip_len, true);
        let (context, compute) = Self::compute(&r, &geom, sr);
        let (audio, st) = mask::render(
            &geom,
            &Reductions::Dynamic {
                context,
                compute: &compute,
            },
            r.link_channels,
            input,
            None,
            offset,
            a,
            b,
        );
        let mut m = Map::new();
        m.insert(
            "cells_reduced_pct".into(),
            json!(round_to(st.cells_reduced_pct(), 2)),
        );
        m.insert(
            "max_reduction_db".into(),
            json!(round_to(st.max_reduction_db, 2)),
        );
        m.insert(
            "mean_reduction_db".into(),
            json!(round_to(st.mean_reduction_db(), 2)),
        );
        m.insert(
            "energy_removed_db".into(),
            json!(round_to(st.energy_removed_db(), 2)),
        );
        m.insert(
            "band_peak_before_db".into(),
            json!(round_to(st.peak_before_db, 2)),
        );
        m.insert(
            "band_peak_after_db".into(),
            json!(round_to(st.peak_after_db, 2)),
        );
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}
