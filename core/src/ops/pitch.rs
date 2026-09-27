//! `pitch_shift`: raise or lower the pitch by a number of semitones, keeping
//! the duration and, by default, the formants (the character of a voice).
//!
//! A phase vocoder with identity phase locking (Laroche and Dolson, "New
//! phase-vocoder techniques for pitch-shifting, harmonizing and other exotic
//! effects", 1999), on the STFT grid anchored to the clip start:
//! - each frame's peaks are found in the channels' summed power, and every bin
//!   belongs to one peak's region, bounded by the lowest bin between two peaks;
//! - each region moves, intact, by the whole number of bins nearest to where
//!   its peak's frequency (measured from the phase advance since the frame
//!   before) should go. The transform is zero-padded to twice the frame, so
//!   that rounding is at most a quarter of the frame's bin. The region's phases
//!   are rotated by an angle that grows, frame after frame, by the change in
//!   frequency times the hop, so the moved partial has exactly the new
//!   frequency and stays coherent between frames;
//! - with `preserve_formants`, each moved bin is scaled by the spectral envelope
//!   (the peaks' levels, joined in dB) at its destination over the envelope at
//!   its source, so the envelope stays where it was while the harmonics move.
//!
//! The rotation carries over from frame to frame, which in a textbook phase
//! vocoder makes every output sample depend on everything before it. Here the
//! rotations restart from zero once in every cell of two seconds on a grid
//! anchored to the clip start, at the quietest frame of the cell, where a
//! restart is least audible (in speech, a pause). Output then depends on at
//! most two cells before it and one after, so the reach is finite (4 s) and a
//! preview equals the same span of the full render. A sound that never pauses
//! (a hum, a held note) can dip for a few tens of milliseconds at a restart.
//!
//! The same moves, rotations and gains apply to every channel, so the
//! differences between the channels (the stereo image, polarity) are kept.
//! A DC offset stays where it is.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::level::{scope_weight, SCOPE_RAMP_S};
use super::{copy_window, typed, with, Descriptor, Op, OpError, RenderOut};
use crate::audio::AudioBuffer;
use crate::dsp::fft::RealFft;
use crate::dsp::stft::{div_floor, StftSize, WOLA_GAIN};
use crate::dsp::window::sqrt_hann_periodic;
use crate::math::{amp_to_db, atan2, cos, exp, ln, pow, power_to_db, round_to, sin, PI};
use crate::scope::Scope;

/// Frame length at 48 kHz (43 ms), scaled with the rate: fine enough to
/// resolve the harmonics of a voice down to about 80 Hz.
const FRAME_AT_48K: usize = 2048;
/// The transform is this many times the frame (zero-padded).
const PAD: usize = 2;
/// The rotations restart once in each cell of this length.
const CELL_S: f64 = 2.0;
/// Keeping the formants never raises a bin by more than this.
const MAX_FORMANT_BOOST_DB: f64 = 24.0;

pub struct PitchShift {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    semitones: f64,
    #[allow(dead_code)] // parsed to check the parameter set against the descriptor
    preserve_formants: bool,
}

#[derive(Deserialize)]
struct Resolved {
    semitones: f64,
    preserve_formants: bool,
    ratio: f64,
}

/// The frame (length and hop), the transform's length and the restart cell,
/// in frames.
#[derive(Clone, Copy)]
struct Grid {
    size: StftSize,
    nfft: usize,
    cell: i64,
}

impl Grid {
    fn bins(&self) -> usize {
        self.nfft / 2 + 1
    }

    /// Where the frame's first sample goes in the transform: centred, so a
    /// frame's phases are measured from its centre up to a sign per bin.
    fn pad_offset(&self) -> usize {
        (self.nfft - self.size.n) / 2
    }
}

fn grid(sr: u32) -> Grid {
    let size = StftSize::scaled(FRAME_AT_48K, sr);
    let cell = ((CELL_S * sr as f64) / size.hop as f64).round().max(1.0) as i64;
    Grid {
        size,
        nfft: PAD * size.n,
        cell,
    }
}

/// `x` wrapped to (−π, π].
fn princarg(x: f64) -> f64 {
    x - 2.0 * PI * (x / (2.0 * PI)).round()
}

/// Sample `i` (absolute) of `src`, whose first sample is absolute `offset`;
/// zero outside the clip or the slice.
#[inline]
fn sample(src: &[f32], offset: i64, clip_len: usize, i: i64) -> f64 {
    let j = i - offset;
    if i >= 0 && (i as usize) < clip_len && j >= 0 && (j as usize) < src.len() {
        src[j as usize] as f64
    } else {
        0.0
    }
}

/// The frame at which the rotations restart in cell `c` (frames
/// `c·cell … (c+1)·cell − 1`): the one with the least energy over all
/// channels, the earliest on a tie.
fn restart_frame(
    g: Grid,
    hann: &[f64],
    input: &AudioBuffer,
    offset: i64,
    clip_len: usize,
    c: i64,
) -> i64 {
    let mut best = (f64::INFINITY, c * g.cell);
    for k in c * g.cell..(c + 1) * g.cell {
        let start = g.size.frame_start(k);
        let mut e = 0.0f64;
        for ch in &input.channels {
            for (m, &w) in hann.iter().enumerate() {
                let x = sample(ch, offset, clip_len, start + m as i64);
                e += x * x * w;
            }
        }
        if e < best.0 {
            best = (e, k);
        }
    }
    best.1
}

/// One channel's spectrum.
#[derive(Clone)]
struct Spectrum {
    re: Vec<f64>,
    im: Vec<f64>,
}

impl Spectrum {
    fn new(bins: usize) -> Self {
        Spectrum {
            re: vec![0.0; bins],
            im: vec![0.0; bins],
        }
    }

    fn power(&self, b: usize) -> f64 {
        self.re[b] * self.re[b] + self.im[b] * self.im[b]
    }
}

/// One region's move: bins `lo..hi` go up by `dk` bins (down if negative),
/// multiplied by `(rot_re + i·rot_im)`.
struct Move {
    lo: usize,
    hi: usize,
    dk: i64,
    rot_re: f64,
    rot_im: f64,
}

/// What a frame does, decided from the `decide` buffer's spectra: the moves,
/// and the gain for each source bin (1 unless keeping the formants).
struct Plan {
    moves: Vec<Move>,
    gain: Vec<f64>,
}

struct Shifter {
    g: Grid,
    ratio: f64,
    formants: bool,
    max_boost_ln: f64,
    fft: RealFft,
    window: Vec<f64>,
    buf_in: Vec<f64>,
    buf_out: Vec<f64>,
    power: Vec<f64>,
    peaks: Vec<usize>,
    log_env: Vec<f64>,
}

impl Shifter {
    fn new(g: Grid, ratio: f64, formants: bool) -> Self {
        let bins = g.bins();
        Shifter {
            g,
            ratio,
            formants,
            max_boost_ln: MAX_FORMANT_BOOST_DB / 20.0 * ln(10.0),
            fft: RealFft::new(g.nfft),
            window: sqrt_hann_periodic(g.size.n),
            buf_in: vec![0.0; g.nfft],
            buf_out: vec![0.0; g.nfft],
            power: vec![0.0; bins],
            peaks: Vec::new(),
            log_env: vec![0.0; bins],
        }
    }

    /// Frame `k` of every channel of `input`, windowed and zero-padded.
    fn analyse(
        &mut self,
        input: &AudioBuffer,
        offset: i64,
        clip_len: usize,
        k: i64,
        out: &mut [Spectrum],
    ) {
        let start = self.g.size.frame_start(k);
        let pad = self.g.pad_offset();
        for (src, s) in input.channels.iter().zip(out.iter_mut()) {
            for (m, &w) in self.window.iter().enumerate() {
                self.buf_in[pad + m] = sample(src, offset, clip_len, start + m as i64) * w;
            }
            self.fft.forward(&self.buf_in, &mut s.re, &mut s.im);
        }
    }

    /// Decide frame `k`'s moves from its spectra (`cur`) and the frame
    /// before's (`prev`), carrying each region's rotation over from
    /// `theta_prev` (unless it `restart`s) into `theta_cur`, both indexed by
    /// source bin.
    fn plan(
        &mut self,
        cur: &[Spectrum],
        prev: &[Spectrum],
        restart: bool,
        theta_prev: &[f64],
        theta_cur: &mut [f64],
    ) -> Plan {
        let bins = self.g.bins();
        for b in 0..bins {
            self.power[b] = cur.iter().map(|s| s.power(b)).sum();
        }
        let p = &self.power;
        let at = |j: i64| {
            if j >= 0 && (j as usize) < bins {
                p[j as usize]
            } else {
                0.0
            }
        };
        let is_peak = |b: usize| {
            let (v, i) = (p[b], b as i64);
            v > 0.0 && v > at(i - 1) && v > at(i - 2) && v > at(i + 1) && v > at(i + 2)
        };
        // Peaks: louder than the two bins on each side. The top bin is never
        // one, and is dropped.
        self.peaks.clear();
        self.peaks.extend((1..bins - 1).filter(|&b| is_peak(b)));
        // The lowest bin between two peaks (the earliest on a tie) starts the
        // upper one's region.
        let valley = |lo: usize, hi: usize| {
            (lo + 1..hi).fold(lo + 1, |best, b| if p[b] < p[best] { b } else { best })
        };
        // DC stays where it is: with its window's lobe, when it stands out.
        let fixed_hi = if is_peak(0) {
            self.peaks.first().map_or(bins - 1, |&pk| valley(0, pk))
        } else {
            1
        };
        let mut moves = Vec::with_capacity(self.peaks.len() + 1);
        moves.push(Move {
            lo: 0,
            hi: fixed_hi,
            dk: 0,
            rot_re: 1.0,
            rot_im: 0.0,
        });
        theta_cur.fill(0.0);
        let hop_rad = 2.0 * PI * self.g.size.hop as f64 / self.g.nfft as f64;
        for (i, &pk) in self.peaks.iter().enumerate() {
            let lo = if i == 0 {
                fixed_hi
            } else {
                valley(self.peaks[i - 1], pk)
            };
            let hi = match self.peaks.get(i + 1) {
                Some(&next) => valley(pk, next),
                None => bins - 1,
            };
            // The peak's frequency (in bins) from its phase advance in the
            // loudest channel there.
            let ch = (0..cur.len()).fold(0, |best, c| {
                if cur[c].power(pk) > cur[best].power(pk) {
                    c
                } else {
                    best
                }
            });
            let (cr, ci) = (cur[ch].re[pk], cur[ch].im[pk]);
            let (pr, pi) = (prev[ch].re[pk], prev[ch].im[pk]);
            let advance = atan2(ci * pr - cr * pi, cr * pr + ci * pi);
            let deviation = princarg(advance - hop_rad * pk as f64);
            let freq_bins = pk as f64 + deviation / hop_rad;
            let shift_bins = (self.ratio - 1.0) * freq_bins;
            let dk = shift_bins.round() as i64;
            let theta = if restart {
                0.0
            } else {
                princarg(theta_prev[pk] + shift_bins * hop_rad)
            };
            theta_cur[lo..hi].fill(theta);
            // Phases are measured from the transform's start, half a
            // transform before the frame's centre: moving a region by an odd
            // number of bins turns it over, so it is turned back.
            let sign = if dk % 2 == 0 { 1.0 } else { -1.0 };
            moves.push(Move {
                lo,
                hi,
                dk,
                rot_re: sign * cos(theta),
                rot_im: sign * sin(theta),
            });
        }
        let mut gain = vec![1.0f64; bins];
        if self.formants && !self.peaks.is_empty() {
            self.envelope();
            let top = bins as i64 - 2;
            for mv in &moves[1..] {
                for s in mv.lo..mv.hi {
                    let d = s as i64 + mv.dk;
                    if d >= 1 && d <= top {
                        let change = self.log_env[d as usize] - self.log_env[s];
                        gain[s] = exp(change.min(self.max_boost_ln));
                    }
                }
            }
        }
        Plan { moves, gain }
    }

    /// The spectral envelope (log amplitude per bin): the peaks' levels,
    /// joined by straight lines in dB, and level beyond the first and last.
    fn envelope(&mut self) {
        let level = |b: usize| 0.5 * ln(self.power[b]);
        let (first, last) = (self.peaks[0], self.peaks[self.peaks.len() - 1]);
        self.log_env[..=first].fill(level(first));
        self.log_env[last..].fill(level(last));
        for w in self.peaks.windows(2) {
            let (b0, b1) = (w[0], w[1]);
            let (l0, l1) = (level(b0), level(b1));
            let span = (b1 - b0) as f64;
            for b in b0..b1 {
                self.log_env[b] = l0 + (l1 - l0) * (b - b0) as f64 / span;
            }
        }
    }

    /// Apply `plan` to one channel's spectrum.
    fn apply(&self, plan: &Plan, x: &Spectrum, y: &mut Spectrum) {
        let top = self.g.bins() as i64 - 2;
        y.re.fill(0.0);
        y.im.fill(0.0);
        for mv in &plan.moves {
            for s in mv.lo..mv.hi {
                let d = s as i64 + mv.dk;
                // Nothing moves into DC or the top bin, or beyond.
                if d > top || d < 0 || (d == 0 && s != 0) {
                    continue;
                }
                let d = d as usize;
                let g = plan.gain[s];
                let (xr, xi) = (x.re[s], x.im[s]);
                y.re[d] += g * (xr * mv.rot_re - xi * mv.rot_im);
                y.im[d] += g * (xr * mv.rot_im + xi * mv.rot_re);
            }
        }
    }

    /// Inverse-transform `y` as frame `k`, window it and add it into `out`,
    /// whose first element is absolute sample `a`.
    fn synthesise(&mut self, y: &Spectrum, k: i64, out: &mut [f64], a: i64) {
        self.fft.inverse(&y.re, &y.im, &mut self.buf_out);
        let start = self.g.size.frame_start(k) - a;
        let pad = self.g.pad_offset();
        let len = out.len() as i64;
        let norm = 1.0 / WOLA_GAIN;
        for (m, &w) in self.window.iter().enumerate() {
            let i = start + m as i64;
            if i >= 0 && i < len {
                out[i as usize] += self.buf_out[pad + m] * w * norm;
            }
        }
    }
}

/// `target` pitch-shifted over absolute samples `[a, b)`, with every decision
/// (restarts, peaks, moves, rotations, envelopes) taken from `decide`
/// (normally the same buffer; both start at absolute sample `offset`).
/// Rotations restart once per `g.cell` frames.
#[allow(clippy::too_many_arguments)]
fn shift(
    r: &Resolved,
    g: Grid,
    decide: &AudioBuffer,
    target: &AudioBuffer,
    offset: i64,
    clip_len: usize,
    a: i64,
    b: i64,
) -> Vec<Vec<f64>> {
    let nch = target.num_channels();
    let mut out = vec![vec![0.0f64; (b - a).max(0) as usize]; nch];
    if a >= b {
        return out;
    }
    let size = g.size;
    let bins = g.bins();
    let mut sh = Shifter::new(g, r.ratio, r.preserve_formants);
    let hann: Vec<f64> = sh.window.iter().map(|w| w * w).collect();

    // Where the rotations restart, from the cell before the first frame's to
    // the last frame's; the chain for the first frame begins at the last
    // restart at or before it.
    let (ka, kb) = size.frames_overlapping(a, b);
    let c_first = div_floor(ka, g.cell) - 1;
    let c_last = div_floor(kb, g.cell);
    let restarts: Vec<i64> = (c_first..=c_last)
        .map(|c| restart_frame(g, &hann, decide, offset, clip_len, c))
        .collect();
    let restart_of = |k: i64| restarts[(div_floor(k, g.cell) - c_first) as usize];
    let first = if restart_of(ka) <= ka {
        restart_of(ka)
    } else {
        restarts[(div_floor(ka, g.cell) - 1 - c_first) as usize]
    };

    let same = std::ptr::eq(decide, target);
    let dch = decide.num_channels();
    let mut prev = vec![Spectrum::new(bins); dch];
    let mut cur = vec![Spectrum::new(bins); dch];
    let mut x = vec![Spectrum::new(bins); nch];
    let mut y = Spectrum::new(bins);
    let mut theta_prev = vec![0.0f64; bins];
    let mut theta_cur = vec![0.0f64; bins];
    sh.analyse(decide, offset, clip_len, first - 1, &mut prev);
    for k in first..=kb {
        sh.analyse(decide, offset, clip_len, k, &mut cur);
        let plan = sh.plan(&cur, &prev, k == restart_of(k), &theta_prev, &mut theta_cur);
        if k >= ka {
            let spectra: &[Spectrum] = if same {
                &cur
            } else {
                sh.analyse(target, offset, clip_len, k, &mut x);
                &x
            };
            for (ch, o) in out.iter_mut().enumerate() {
                sh.apply(&plan, &spectra[ch], &mut y);
                sh.synthesise(&y, k, o, a);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
        std::mem::swap(&mut theta_prev, &mut theta_cur);
    }
    out
}

/// `target` over `[a, b)`, pitch-shifted inside the scope (fading over 5 ms
/// at its interior edges) and copied exactly outside it.
#[allow(clippy::too_many_arguments)]
fn render_with(
    r: &Resolved,
    scope: &Scope,
    decide: &AudioBuffer,
    target: &AudioBuffer,
    offset: i64,
    a: i64,
    b: i64,
    clip_len: usize,
) -> AudioBuffer {
    let mut out = copy_window(target, offset, a, b);
    if r.semitones == 0.0 {
        return out;
    }
    let sr = target.sample_rate;
    let (s0, s1) = scope.samples(sr, clip_len);
    let (s0, s1) = (s0 as i64, s1 as i64);
    let (lo, hi) = (a.max(s0), b.min(s1));
    if lo >= hi {
        return out;
    }
    let shifted = shift(r, grid(sr), decide, target, offset, clip_len, lo, hi);
    let ramp_len = (SCOPE_RAMP_S * sr as f64).round() as i64;
    for (ch, y) in out.channels.iter_mut().zip(&shifted) {
        for (j, &v) in y.iter().enumerate() {
            let i = lo + j as i64;
            let w = scope_weight(i, s0, s1, clip_len as i64, ramp_len);
            let x = &mut ch[(i - a) as usize];
            if w == 1.0 {
                *x = v as f32;
            } else if w > 0.0 {
                *x = (*x as f64 + (v - *x as f64) * w) as f32;
            }
        }
    }
    out
}

impl Op for PitchShift {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        _scope: &Scope,
        _input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        let p: Params = typed(params)?;
        let ratio = round_to(pow(2.0, p.semitones / 12.0), 6);
        Ok(with(params, json!({ "ratio": ratio })))
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        match typed::<Resolved>(resolved) {
            Ok(r) if r.semitones != 0.0 => {
                let g = grid(sr);
                g.size.n + 2 * g.cell as usize * g.size.hop
            }
            _ => 0,
        }
    }

    fn render_linear(
        &self,
        resolved: &Value,
        scope: &Scope,
        decide: &AudioBuffer,
        target: &AudioBuffer,
    ) -> Result<Option<AudioBuffer>, OpError> {
        let r: Resolved = typed(resolved)?;
        let len = target.len();
        Ok(Some(render_with(
            &r, scope, decide, target, 0, 0, len as i64, len,
        )))
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
        let r: Resolved = typed(resolved)?;
        let audio = render_with(&r, scope, input, input, offset, a, b, clip_len);
        let before = copy_window(input, offset, a, b);
        let (mut e0, mut e1, mut p0, mut p1) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (x, y) in before.channels.iter().zip(&audio.channels) {
            for (&u, &v) in x.iter().zip(y) {
                let (u, v) = (u as f64, v as f64);
                e0 += u * u;
                e1 += v * v;
                p0 = p0.max(u.abs());
                p1 = p1.max(v.abs());
            }
        }
        let mut m = Map::new();
        m.insert("ratio".into(), json!(r.ratio));
        m.insert(
            "level_change_db".into(),
            json!(if e0 > 0.0 {
                round_to(power_to_db(e1 / e0), 2)
            } else {
                0.0
            }),
        );
        m.insert("peak_before_dbfs".into(), json!(round_to(amp_to_db(p0), 2)));
        m.insert("peak_after_dbfs".into(), json!(round_to(amp_to_db(p1), 2)));
        Ok(RenderOut {
            audio,
            measurements: m,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SR: u32 = 48000;

    fn resolved(semitones: f64, preserve_formants: bool) -> Resolved {
        Resolved {
            semitones,
            preserve_formants,
            ratio: pow(2.0, semitones / 12.0),
        }
    }

    fn sine(f: f64, secs: f64, amp: f64) -> Vec<f32> {
        (0..(secs * SR as f64) as usize)
            .map(|i| (amp * sin(2.0 * PI * f * i as f64 / SR as f64)) as f32)
            .collect()
    }

    fn render_mono(x: &[f32], r: &Resolved) -> Vec<f32> {
        let buf = AudioBuffer::mono(SR, x.to_vec());
        let len = x.len();
        render_with(r, &Scope::Clip, &buf, &buf, 0, 0, len as i64, len).channels[0].clone()
    }

    /// The frequency of a steady tone over `[i0, i1)`, from its upward zero
    /// crossings (interpolated between samples).
    fn frequency(x: &[f32], i0: usize, i1: usize) -> f64 {
        let mut crossings = Vec::new();
        for i in i0.max(1)..i1 {
            let (p, q) = (x[i - 1] as f64, x[i] as f64);
            if p < 0.0 && q >= 0.0 {
                crossings.push(i as f64 - 1.0 + p / (p - q));
            }
        }
        let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
        (crossings.len() - 1) as f64 * SR as f64 / (last - first)
    }

    /// Amplitude of the component at `f` over `[i0, i1)`.
    fn amplitude_at(x: &[f32], f: f64, i0: usize, i1: usize) -> f64 {
        let (mut c, mut s) = (0.0, 0.0);
        for (i, &v) in x.iter().enumerate().take(i1).skip(i0) {
            let ph = 2.0 * PI * f * i as f64 / SR as f64;
            c += v as f64 * cos(ph);
            s += v as f64 * sin(ph);
        }
        2.0 * (c * c + s * s).sqrt() / (i1 - i0) as f64
    }

    /// Restart frames of the cells overlapping a buffer.
    fn restarts(x: &[f32]) -> Vec<i64> {
        let g = grid(SR);
        let buf = AudioBuffer::mono(SR, x.to_vec());
        let hann: Vec<f64> = sqrt_hann_periodic(g.size.n).iter().map(|w| w * w).collect();
        let cells = div_floor(x.len() as i64 / g.size.hop as i64, g.cell);
        (-1..=cells)
            .map(|c| restart_frame(g, &hann, &buf, 0, x.len(), c))
            .collect()
    }

    #[test]
    fn a_tone_moves_by_the_ratio_and_keeps_its_level() {
        let x = sine(440.0, 3.5, 0.25);
        // Measured between restarts: here at the start and after the end,
        // where the tone is quietest.
        let (i0, i1) = (14400, 144000);
        let hop = grid(SR).size.hop as i64;
        assert!(restarts(&x)
            .iter()
            .all(|&k| k * hop < i0 as i64 - 4096 || k * hop > i1 as i64 + 4096));
        for semitones in [-12.0, -5.0, -0.3, 2.0, 7.0, 12.0] {
            let r = resolved(semitones, false);
            let y = render_mono(&x, &r);
            let f = frequency(&y, i0, i1);
            let want = 440.0 * r.ratio;
            assert!(
                (f - want).abs() < 0.01,
                "{semitones:+} st: {f:.4} Hz, want {want:.4}"
            );
            let level = amp_to_db(amplitude_at(&y, want, i0, i1) / 0.25);
            assert!(level.abs() < 0.5, "{semitones:+} st: level {level:.2} dB");
            // (Closer than a semitone, the new tone leaks into the measurement.)
            if semitones.abs() >= 1.0 {
                assert!(
                    amp_to_db(amplitude_at(&y, 440.0, i0, i1) / 0.25) < -40.0,
                    "{semitones:+} st: the original tone remains"
                );
            }
        }
    }

    #[test]
    fn previews_around_restarts_equal_the_full_render() {
        // Pauses in each of three cells put restarts inside the clip.
        let mut x: Vec<f32> = voiced(130.0, 0.03, 1.0)
            .iter()
            .chain(&voiced(170.0, 0.02, 1.0))
            .chain(&voiced(110.0, 0.04, 1.0))
            .chain(&voiced(150.0, 0.01, 1.0))
            .copied()
            .collect();
        for p in [1.3, 3.1, 4.9] {
            let s = (p * SR as f64) as usize;
            x[s..s + 4800].fill(0.0);
        }
        let hop = grid(SR).size.hop as i64;
        let inside: Vec<i64> = restarts(&x)
            .into_iter()
            .map(|k| k * hop)
            .filter(|&c| c > 0 && c < x.len() as i64)
            .collect();
        assert_eq!(inside.len(), 3, "restarts {inside:?}");
        let buf = AudioBuffer::new(SR, vec![x.clone(), x.iter().map(|v| 0.5 * v).collect()]);
        let reg = crate::ops::registry();
        let p = reg
            .validate("pitch_shift", 1, &json!({"semitones": -2.5}), &Scope::Clip)
            .unwrap();
        let op = reg.get("pitch_shift", 1).unwrap();
        let step = crate::engine::RenderStep {
            op: "pitch_shift".into(),
            op_version: 1,
            resolved: op.resolve(&p, &Scope::Clip, &buf).unwrap(),
            scope: Scope::Clip,
        };
        let steps = std::slice::from_ref(&step);
        let whole = crate::engine::render_full(&buf, steps).unwrap().audio;
        for &c in &inside {
            for (a, b) in [
                (c - 3000, c + 3000),
                (c, c + 1),
                (c - 1, c),
                (c + 700, c + 20000),
                (c - 40000, c - 39000),
            ] {
                let w = crate::engine::render_window(&buf, steps, a, b).unwrap();
                assert_eq!(w, whole.extract_padded(a, b), "preview {a}..{b}");
            }
        }
    }

    #[test]
    fn components_decided_by_the_mixture_sum_to_its_render() {
        let voice = voiced(140.0, 0.02, 1.0);
        let tone = sine(2500.0, 1.5, 0.05);
        let mix: Vec<f32> = voice.iter().zip(&tone).map(|(a, b)| a + b).collect();
        let (mix, voice, tone) = (
            AudioBuffer::mono(SR, mix),
            AudioBuffer::mono(SR, voice),
            AudioBuffer::mono(SR, tone),
        );
        let op = crate::ops::registry().get("pitch_shift", 1).unwrap();
        let resolved =
            json!({"semitones": 4, "preserve_formants": true, "ratio": pow(2.0, 4.0 / 12.0)});
        let scope = Scope::TimeRange { t0: 0.3, t1: 1.2 };
        let len = mix.len();
        let whole = op
            .render(&resolved, &scope, &mix, 0, 0, len as i64, len)
            .unwrap()
            .audio;
        let part = |c: &AudioBuffer| {
            op.render_linear(&resolved, &scope, &mix, c)
                .unwrap()
                .unwrap()
        };
        let (v, t) = (part(&voice), part(&tone));
        let worst = (0..len)
            .map(|i| {
                (v.channels[0][i] as f64 + t.channels[0][i] as f64 - whole.channels[0][i] as f64)
                    .abs()
            })
            .fold(0.0, f64::max);
        assert!(
            amp_to_db(worst) < -100.0,
            "components differ by {} dB",
            amp_to_db(worst)
        );
    }

    #[test]
    fn restarts_fall_on_the_quietest_frame() {
        // A tone with a 100 ms pause at 2.5 s, in the second cell.
        let mut x = sine(300.0, 4.0, 0.5);
        x[120000..124800].fill(0.0);
        let hop = grid(SR).size.hop as i64;
        let r = restarts(&x);
        assert!(
            r.iter().any(|&k| (120000..124800).contains(&(k * hop))),
            "no restart in the pause: {r:?}"
        );
    }

    /// Harmonics of f0 (with a 5 Hz vibrato of `depth`) under three fixed
    /// formants, 1.5 s; `scale` moves f0 only, which is what a pitch shift
    /// that keeps the formants should produce.
    fn voiced(f0: f64, depth: f64, scale: f64) -> Vec<f32> {
        let env = |f: f64| {
            [(500.0, 1.0), (1500.0, 0.5), (2500.0, 0.3)]
                .iter()
                .map(|&(c, a)| a / (1.0 + ((f - c) / 120.0) * ((f - c) / 120.0)))
                .sum::<f64>()
                + 0.01
        };
        let mut ph = 0.0;
        (0..SR as usize * 3 / 2)
            .map(|i| {
                let t = i as f64 / SR as f64;
                let f = scale * f0 * (1.0 + depth * sin(2.0 * PI * 5.0 * t));
                ph += 2.0 * PI * f / SR as f64;
                (1..=30)
                    .map(|h| 0.02 * env(h as f64 * f) * sin(h as f64 * ph))
                    .sum::<f64>() as f32
            })
            .collect()
    }

    /// Log-spectral distance (dB) between two signals: 43 ms frames, 80 Hz –
    /// 5 kHz, a floor 60 dB below each frame's peak, the mean offset removed.
    fn lsd(a: &[f32], b: &[f32]) -> f64 {
        let (n, hop) = (2048usize, 512usize);
        let w = sqrt_hann_periodic(n);
        let mut fft = RealFft::new(n);
        let bins = n / 2 + 1;
        let mut spec = |x: &[f32], k: usize| {
            let fr: Vec<f64> = (0..n).map(|m| x[k * hop + m] as f64 * w[m]).collect();
            let (mut re, mut im) = (vec![0.0; bins], vec![0.0; bins]);
            fft.forward(&fr, &mut re, &mut im);
            (0..bins)
                .map(|b| re[b] * re[b] + im[b] * im[b])
                .collect::<Vec<f64>>()
        };
        let (b0, b1) = (80 * n / SR as usize, 5000 * n / SR as usize);
        let mut diffs = Vec::new();
        for k in 4..(a.len() - n) / hop - 4 {
            let (sa, sb) = (spec(a, k), spec(b, k));
            let peak = sa.iter().chain(&sb).fold(0.0f64, |m, &v| m.max(v));
            for bb in b0..b1 {
                diffs.push(
                    power_to_db(sa[bb].max(peak * 1e-6)) - power_to_db(sb[bb].max(peak * 1e-6)),
                );
            }
        }
        let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
        (diffs.iter().map(|d| (d - mean) * (d - mean)).sum::<f64>() / diffs.len() as f64).sqrt()
    }

    #[test]
    fn a_voiced_sound_comes_out_close_to_the_ideal_shift() {
        for semitones in [-5.0, 3.0] {
            let r = resolved(semitones, true);
            let x = voiced(140.0, 0.02, 1.0);
            let ideal = voiced(140.0, 0.02, r.ratio);
            let kept = render_mono(&x, &r);
            let moved = render_mono(&x, &resolved(semitones, false));
            let (before, after, without) =
                (lsd(&x, &ideal), lsd(&kept, &ideal), lsd(&moved, &ideal));
            assert!(
                after < 4.0 && after < before / 3.0 && after < without / 2.0,
                "{semitones:+} st: {after:.2} dB from the ideal (unshifted {before:.2}, formants moved {without:.2})"
            );
        }
    }

    #[test]
    fn the_formants_stay_or_move_with_the_pitch() {
        // Harmonics of 150 Hz shaped by one formant at 1 kHz.
        let len = SR as usize * 2;
        let f0 = 150.0;
        let env = |f: f64| 1.0 / (1.0 + ((f - 1000.0) / 150.0) * ((f - 1000.0) / 150.0));
        let x: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f64 / SR as f64;
                (1..40)
                    .map(|h| 0.05 * env(h as f64 * f0) * sin(2.0 * PI * h as f64 * f0 * t))
                    .sum::<f64>() as f32
            })
            .collect();
        // The loudest harmonic of the output, as a frequency.
        let loudest = |y: &[f32], f0: f64| {
            (1..40)
                .map(|h| (h as f64 * f0, amplitude_at(y, h as f64 * f0, 24000, 72000)))
                .fold((0.0, 0.0), |m, v| if v.1 > m.1 { v } else { m })
                .0
        };
        let up = resolved(4.0, true);
        let f0_up = f0 * up.ratio;
        let kept = loudest(&render_mono(&x, &up), f0_up);
        assert!(
            (kept - 1000.0).abs() < f0_up / 2.0,
            "formant moved to {kept} Hz"
        );
        let moved = loudest(&render_mono(&x, &resolved(4.0, false)), f0_up);
        assert!(
            (moved - 1000.0 * up.ratio).abs() < f0_up / 2.0,
            "formant at {moved} Hz, want it moved with the pitch"
        );
    }

    #[test]
    fn channels_keep_their_differences() {
        let x = sine(523.0, 1.5, 0.3);
        let neg: Vec<f32> = x.iter().map(|v| -v).collect();
        let buf = AudioBuffer::new(SR, vec![x.clone(), neg]);
        let len = x.len();
        let r = resolved(-3.0, true);
        let out = render_with(&r, &Scope::Clip, &buf, &buf, 0, 0, len as i64, len);
        for (l, r) in out.channels[0].iter().zip(&out.channels[1]) {
            assert_eq!(l.to_bits(), (-r).to_bits());
        }
        assert_ne!(out.channels[0], x);
    }

    #[test]
    fn dc_stays_where_it_is() {
        let x: Vec<f32> = sine(400.0, 2.0, 0.2).iter().map(|v| v + 0.1).collect();
        let y = render_mono(&x, &resolved(5.0, true));
        let mean = |v: &[f32]| v[24000..72000].iter().map(|&s| s as f64).sum::<f64>() / 48000.0;
        assert!((mean(&y) - 0.1).abs() < 1e-3, "mean {}", mean(&y));
    }
}
