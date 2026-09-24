//! The STFT-mask engine shared by `noise_reduce`, `line_reduce`, `band_cut`
//! and `spectral_compressor`.
//!
//! Each operation supplies a reduction in dB (≤ 0) for every time–frequency
//! cell in its scope. The engine renders by subtraction:
//! `residual = ISTFT((1 − g)·X)`, `output = input − residual`. Cells that are
//! not reduced contribute exactly zero, so:
//! - samples outside the scope (plus one window) are bit-identical;
//! - identity settings are bit-identical;
//! - the residual is exactly what was removed;
//! - with the frame grid anchored to the clip start and every smoothing of
//!   finite reach, a preview equals the same span of the final render.

use crate::audio::AudioBuffer;
use crate::dsp::fft::RealFft;
use crate::dsp::stft::{div_ceil, Stft, StftSize};
use crate::dsp::window::{ramp, sqrt_hann_periodic};
use crate::math::{db_to_amp, power_to_db};
use crate::scope::Scope;

/// Reductions smaller than this snap to exactly zero.
pub const SNAP_DB: f32 = -0.001;
const TIME_TAPER_FRAMES: usize = 2;

/// Where in the time–frequency plane an operation may act.
#[derive(Clone, Debug)]
pub struct Geometry {
    pub size: StftSize,
    pub sample_rate: u32,
    pub clip_len: usize,
    pub k_first: i64,
    pub k_last: i64,
    pub taper_left: bool,
    pub taper_right: bool,
    pub b_lo: usize,
    pub b_hi: usize,
    pub freq_taper: usize,
    pub taper_low: bool,
    pub taper_high: bool,
}

impl Geometry {
    /// `band`: whether the scope's frequency band restricts where reductions apply.
    pub fn new(
        scope: &Scope,
        size: StftSize,
        sample_rate: u32,
        clip_len: usize,
        band: bool,
    ) -> Self {
        let (s0, s1) = scope.samples(sample_rate, clip_len);
        let half = (size.n / 2) as i64;
        // A scope edge at the clip boundary extends to every frame touching the
        // clip; interior edges fade over two frames.
        let taper_left = s0 > 0;
        let taper_right = s1 < clip_len;
        let e0 = if taper_left { s0 as i64 } else { -half };
        let e1 = if taper_right {
            s1 as i64
        } else {
            clip_len as i64 + half
        };
        let hop = size.hop as i64;
        let k_first = div_ceil(e0, hop);
        let k_last = div_ceil(e1, hop) - 1;
        let bins = size.bins();
        let bin_hz = size.bin_hz(sample_rate);
        let (mut b_lo, mut b_hi) = (0usize, bins - 1);
        let (mut taper_low, mut taper_high) = (false, false);
        if band {
            if let Some((f_lo, f_hi)) = scope.band() {
                if f_lo > 0.0 {
                    b_lo = ((f_lo / bin_hz).ceil() as usize).min(bins - 1);
                    taper_low = true;
                }
                if f_hi < sample_rate as f64 / 2.0 {
                    b_hi = ((f_hi / bin_hz).floor() as usize).min(bins - 1);
                    taper_high = true;
                }
            }
        }
        let width = b_hi.saturating_sub(b_lo);
        Geometry {
            size,
            sample_rate,
            clip_len,
            k_first,
            k_last,
            taper_left,
            taper_right,
            b_lo,
            b_hi: b_hi.max(b_lo),
            freq_taper: (width / 4).min(2),
            taper_low,
            taper_high,
        }
    }

    /// Weight (0–1) of a reduction at frame `k`, bin `b`.
    pub fn weight(&self, k: i64, b: usize) -> f64 {
        if k < self.k_first || k > self.k_last || b < self.b_lo || b > self.b_hi {
            return 0.0;
        }
        let mut w = 1.0;
        let t = TIME_TAPER_FRAMES as i64;
        if self.taper_left && k - self.k_first < t {
            w *= ramp((k - self.k_first) as usize, TIME_TAPER_FRAMES);
        }
        if self.taper_right && self.k_last - k < t {
            w *= ramp((self.k_last - k) as usize, TIME_TAPER_FRAMES);
        }
        let f = self.freq_taper;
        if f > 0 {
            if self.taper_low && b - self.b_lo < f {
                w *= ramp(b - self.b_lo, f);
            }
            if self.taper_high && self.b_hi - b < f {
                w *= ramp(self.b_hi - b, f);
            }
        }
        w
    }
}

/// Cell levels (dB, relative to a full-scale sine) for a block of frames.
pub struct Levels {
    pub k0: i64,
    pub frames: usize,
    pub bins: usize,
    pub db: Vec<f32>,
}

impl Levels {
    #[inline]
    pub fn at(&self, k: i64, b: usize) -> f32 {
        self.db[(k - self.k0) as usize * self.bins + b]
    }

    pub fn contains(&self, k: i64) -> bool {
        k >= self.k0 && k < self.k0 + self.frames as i64
    }
}

/// How an operation supplies its reductions.
pub enum Reductions<'a> {
    /// The same reduction per bin for every frame in scope (dB, ≤ 0).
    Static(&'a [f32]),
    /// Computed from cell levels: for frames `k0..=k1` return `(k1−k0+1) × bins`
    /// reductions. Levels are provided for `k0 − context ..= k1 + context`.
    Dynamic {
        context: usize,
        compute: &'a dyn Fn(&Levels, i64, i64) -> Vec<f32>,
    },
}

/// Accumulated over cells whose frame is centred in the rendered range.
#[derive(Clone, Debug, Default)]
pub struct MaskStats {
    pub cells: u64,
    pub cells_reduced: u64,
    pub sum_reduction_db: f64,
    pub max_reduction_db: f64,
    pub energy: f64,
    pub energy_removed: f64,
    pub peak_before_db: f64,
    pub peak_after_db: f64,
}

impl MaskStats {
    pub fn cells_reduced_pct(&self) -> f64 {
        if self.cells == 0 {
            0.0
        } else {
            100.0 * self.cells_reduced as f64 / self.cells as f64
        }
    }

    pub fn mean_reduction_db(&self) -> f64 {
        if self.cells_reduced == 0 {
            0.0
        } else {
            self.sum_reduction_db / self.cells_reduced as f64
        }
    }

    pub fn energy_removed_db(&self) -> f64 {
        if self.energy <= 0.0 || self.energy_removed <= 0.0 {
            crate::math::DB_FLOOR
        } else {
            power_to_db(self.energy_removed / self.energy)
        }
    }
}

/// Input samples an output sample can depend on (each side).
pub fn radius(size: StftSize, context_frames: usize) -> usize {
    size.n + (context_frames + 1) * size.hop
}

struct Analyser {
    fft: RealFft,
    window: Vec<f64>,
    norm: f64,
    frame: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
}

impl Analyser {
    fn new(size: StftSize) -> Self {
        let window = sqrt_hann_periodic(size.n);
        let wsum: f64 = window.iter().sum();
        Analyser {
            fft: RealFft::new(size.n),
            norm: 1.0 / ((wsum / 2.0) * (wsum / 2.0)),
            window,
            frame: vec![0.0; size.n],
            re: vec![0.0; size.bins()],
            im: vec![0.0; size.bins()],
        }
    }

    /// Analyse frame `k` of channel `ch` (absolute positions; outside the clip
    /// or the provided input reads as zero).
    fn analyse(
        &mut self,
        size: StftSize,
        input: &AudioBuffer,
        ch: usize,
        offset: i64,
        clip_len: usize,
        k: i64,
    ) {
        let start = size.frame_start(k);
        let src = &input.channels[ch];
        for m in 0..size.n {
            let i = start + m as i64;
            let j = i - offset;
            let x = if i >= 0 && (i as usize) < clip_len && j >= 0 && (j as usize) < src.len() {
                src[j as usize] as f64
            } else {
                0.0
            };
            self.frame[m] = x * self.window[m];
        }
        self.fft.forward(&self.frame, &mut self.re, &mut self.im);
    }

    fn level_db(&self, b: usize) -> f32 {
        power_to_db((self.re[b] * self.re[b] + self.im[b] * self.im[b]) * self.norm) as f32
    }
}

/// Cell levels for frames `k0..=k1`. `channel = None` links channels (the
/// loudest channel sets each cell's level).
pub fn levels(
    size: StftSize,
    input: &AudioBuffer,
    offset: i64,
    clip_len: usize,
    k0: i64,
    k1: i64,
    channel: Option<usize>,
) -> Levels {
    let bins = size.bins();
    let frames = (k1 - k0 + 1).max(0) as usize;
    let mut db = vec![crate::math::DB_FLOOR as f32; frames * bins];
    let mut an = Analyser::new(size);
    let chans: Vec<usize> = match channel {
        Some(c) => vec![c],
        None => (0..input.num_channels()).collect(),
    };
    for (fi, k) in (k0..=k1).enumerate() {
        for &ch in &chans {
            an.analyse(size, input, ch, offset, clip_len, k);
            for b in 0..bins {
                let v = an.level_db(b);
                let cell = &mut db[fi * bins + b];
                if v > *cell {
                    *cell = v;
                }
            }
        }
    }
    Levels {
        k0,
        frames,
        bins,
        db,
    }
}

/// Render absolute output samples `[a, b)`. Levels (and so the mask) come from
/// `decide` when given, otherwise from `input`; `decide` shares `input`'s offset.
pub fn render(
    geom: &Geometry,
    reductions: &Reductions<'_>,
    linked: bool,
    input: &AudioBuffer,
    decide: Option<&AudioBuffer>,
    offset: i64,
    a: i64,
    b: i64,
) -> (AudioBuffer, MaskStats) {
    let size = geom.size;
    let bins = size.bins();
    let clip_len = geom.clip_len;
    let n_out = (b - a).max(0) as usize;
    let nch = input.num_channels();
    let mut residual = vec![vec![0.0f64; n_out]; nch];
    let mut stats = MaskStats {
        peak_before_db: crate::math::DB_FLOOR,
        peak_after_db: crate::math::DB_FLOOR,
        ..Default::default()
    };

    let (ka, kb) = size.frames_overlapping(a, b);
    let k_start = ka.max(geom.k_first);
    let k_end = kb.min(geom.k_last);
    let (kc_lo, kc_hi) = size.frames_centred_in(a, b);

    if k_start <= k_end {
        let context = match reductions {
            Reductions::Static(_) => 0,
            Reductions::Dynamic { context, .. } => *context,
        };
        let chunk = 512.max(4 * context);
        let groups: Vec<Option<usize>> = if linked {
            vec![None]
        } else {
            (0..nch).map(Some).collect()
        };
        let mut an = Analyser::new(size);
        let mut stft = Stft::new(size);
        let mut yr = vec![0.0f64; bins];
        let mut yi = vec![0.0f64; bins];
        for group in groups {
            let chans: Vec<usize> = match group {
                Some(c) => vec![c],
                None => (0..nch).collect(),
            };
            let mut c0 = k_start;
            while c0 <= k_end {
                let c1 = (c0 + chunk as i64 - 1).min(k_end);
                let red: Vec<f32> = match reductions {
                    Reductions::Static(per_bin) => per_bin.repeat((c1 - c0 + 1) as usize),
                    Reductions::Dynamic { context, compute } => {
                        let lv = levels(
                            size,
                            decide.unwrap_or(input),
                            offset,
                            clip_len,
                            c0 - *context as i64,
                            c1 + *context as i64,
                            group,
                        );
                        compute(&lv, c0, c1)
                    }
                };
                for (fi, k) in (c0..=c1).enumerate() {
                    // Weighted, snapped reductions for this frame.
                    let mut row = vec![0.0f32; bins];
                    let mut any = false;
                    for bb in 0..bins {
                        let r = red[fi * bins + bb].min(0.0);
                        if r == 0.0 {
                            continue;
                        }
                        let w = geom.weight(k, bb);
                        let v = (r as f64 * w) as f32;
                        if v < SNAP_DB {
                            row[bb] = v;
                            any = true;
                        }
                    }
                    let counted = k >= kc_lo && k <= kc_hi;
                    if !any && !counted {
                        continue;
                    }
                    for &ch in &chans {
                        an.analyse(size, input, ch, offset, clip_len, k);
                        if counted {
                            for bb in 0..bins {
                                if geom.weight(k, bb) <= 0.0 {
                                    continue;
                                }
                                let p = an.re[bb] * an.re[bb] + an.im[bb] * an.im[bb];
                                let lvl = power_to_db(p * an.norm);
                                let r = row[bb] as f64;
                                let g = db_to_amp(r);
                                stats.cells += 1;
                                stats.energy += p;
                                stats.energy_removed += p * (1.0 - g) * (1.0 - g);
                                stats.peak_before_db = stats.peak_before_db.max(lvl);
                                stats.peak_after_db = stats.peak_after_db.max(lvl + r);
                                if r < 0.0 {
                                    stats.cells_reduced += 1;
                                    stats.sum_reduction_db += r;
                                    stats.max_reduction_db = stats.max_reduction_db.min(r);
                                }
                            }
                        }
                        if any {
                            for bb in 0..bins {
                                let r = row[bb];
                                if r == 0.0 {
                                    yr[bb] = 0.0;
                                    yi[bb] = 0.0;
                                } else {
                                    let keep = 1.0 - db_to_amp(r as f64);
                                    yr[bb] = an.re[bb] * keep;
                                    yi[bb] = an.im[bb] * keep;
                                }
                            }
                            stft.synthesise_add(&yr, &yi, k, &mut residual[ch], a);
                        }
                    }
                }
                c0 = c1 + 1;
            }
        }
    }

    let mut out = super::copy_window(input, offset, a, b);
    for (ch, res) in residual.iter().enumerate() {
        for (x, &r) in out.channels[ch].iter_mut().zip(res) {
            if r != 0.0 {
                *x = (*x as f64 - r) as f32;
            }
        }
    }
    (out, stats)
}
