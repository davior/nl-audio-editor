//! Time edits: stretches removed from the output, and silence inserted into it.
//!
//! Processing steps always see the whole original recording, so their scopes
//! never move and their residuals stay exact. Time edits are steps like any
//! other (previewed, accepted, undone, logged), but they leave the processing
//! as it is: they are applied to its result, just before the final limiter.
//! Every position is a sample of the original recording, so the order of the
//! edits does not change the output, except that silences inserted at the same
//! point follow the order they were made in.

use serde_json::{json, Value};

use crate::audio::AudioBuffer;
use crate::dsp::window::ramp;
use crate::engine::RenderStep;
use crate::math::round_to;

pub const REMOVE: &str = "remove_time";
pub const INSERT: &str = "insert_silence";

/// Whether an operation is a time edit.
pub fn is_edit(op: &str) -> bool {
    op == REMOVE || op == INSERT
}

/// One edit, in samples of the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Samples `[s0, s1)` are left out; the audio fades out over `fade`
    /// samples before the join and back in after it.
    Remove { s0: usize, s1: usize, fade: usize },
    /// `len` samples of silence at `at`; the audio either side fades over `fade` samples.
    Insert { at: usize, len: usize, fade: usize },
}

/// A piece of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Original samples `[src0, src1)`, faded in and out over the given lengths.
    Kept {
        src0: usize,
        src1: usize,
        fade_in: usize,
        fade_out: usize,
    },
    /// `len` samples of silence, inserted at original sample `at`.
    Silence { at: usize, len: usize },
}

/// Where an edit shows in the output.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// Original samples `[from, to)` were left out here.
    Removed { from: usize, to: usize, out: usize },
    /// `len` samples of silence, inserted at original sample `at`, start here.
    Inserted { at: usize, len: usize, out: usize },
}

impl Mark {
    /// Output sample the mark sits at.
    pub fn out(&self) -> usize {
        match *self {
            Mark::Removed { out, .. } | Mark::Inserted { out, .. } => out,
        }
    }
}

/// How the output is assembled from the original.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pieces: Vec<Piece>,
    pub clip_len: usize,
}

fn field(v: &Value, k: &str) -> usize {
    v[k].as_u64().unwrap_or(0) as usize
}

/// The time edits among a stack's render steps, in stack order.
pub fn edits(steps: &[RenderStep]) -> Vec<Edit> {
    steps
        .iter()
        .filter_map(|s| match s.op.as_str() {
            REMOVE => Some(Edit::Remove {
                s0: field(&s.resolved, "s0"),
                s1: field(&s.resolved, "s1"),
                fade: field(&s.resolved, "fade_samples"),
            }),
            INSERT => Some(Edit::Insert {
                at: field(&s.resolved, "at_sample"),
                len: field(&s.resolved, "samples"),
                fade: field(&s.resolved, "fade_samples"),
            }),
            _ => None,
        })
        .collect()
}

/// Lay out the output of a `clip_len`-sample recording under `edits`.
///
/// - Removed stretches that overlap or touch are merged; a merged stretch
///   fades with the largest fade of those it joins.
/// - A silence inserted inside a removed stretch goes where the stretch was.
/// - Kept audio fades out before, and in after, every join and every inserted
///   silence; at the ends of the recording only where an edit made that end.
pub fn layout(edits: &[Edit], clip_len: usize) -> Layout {
    let mut removed: Vec<(usize, usize, usize)> = edits
        .iter()
        .filter_map(|e| match *e {
            Edit::Remove { s0, s1, fade } => Some((s0.min(clip_len), s1.min(clip_len), fade)),
            _ => None,
        })
        .filter(|r| r.1 > r.0)
        .collect();
    removed.sort_unstable();
    let mut merged: Vec<(usize, usize, usize)> = Vec::new();
    for (a, b, f) in removed {
        match merged.last_mut() {
            Some(last) if a <= last.1 => {
                last.1 = last.1.max(b);
                last.2 = last.2.max(f);
            }
            _ => merged.push((a, b, f)),
        }
    }
    let mut inserts: Vec<(usize, usize, usize)> = edits
        .iter()
        .filter_map(|e| match *e {
            Edit::Insert { at, len, fade } if len > 0 => Some((at.min(clip_len), len, fade)),
            _ => None,
        })
        .collect();
    for i in inserts.iter_mut() {
        if let Some(r) = merged.iter().find(|r| r.0 < i.0 && i.0 < r.1) {
            i.0 = r.0;
        }
    }
    // Stable: silences at the same point keep the order they were made in.
    inserts.sort_by_key(|i| i.0);

    // The kept stretches, then the silences placed among them.
    let mut kept = Vec::new();
    let mut cur = 0;
    for &(a, b, _) in &merged {
        if a > cur {
            kept.push((cur, a));
        }
        cur = b;
    }
    if cur < clip_len {
        kept.push((cur, clip_len));
    }
    // (piece, fade of the silence) before fades are worked out.
    let mut raw: Vec<(Piece, usize)> = Vec::new();
    let mut j = 0;
    let silence = |i: &(usize, usize, usize)| (Piece::Silence { at: i.0, len: i.1 }, i.2);
    let kept_piece = |src0, src1| Piece::Kept {
        src0,
        src1,
        fade_in: 0,
        fade_out: 0,
    };
    for &(k0, k1) in &kept {
        while j < inserts.len() && inserts[j].0 <= k0 {
            raw.push(silence(&inserts[j]));
            j += 1;
        }
        let mut from = k0;
        while j < inserts.len() && inserts[j].0 < k1 {
            raw.push((kept_piece(from, inserts[j].0), 0));
            from = inserts[j].0;
            raw.push(silence(&inserts[j]));
            j += 1;
        }
        raw.push((kept_piece(from, k1), 0));
    }
    while j < inserts.len() {
        raw.push(silence(&inserts[j]));
        j += 1;
    }

    // Fades: from a removed stretch that ends where the kept audio starts (or
    // starts where it ends), and from a silence next to it.
    let gap_ending_at = |s: usize| merged.iter().find(|r| r.1 == s).map_or(0, |r| r.2);
    let gap_starting_at = |s: usize| merged.iter().find(|r| r.0 == s).map_or(0, |r| r.2);
    let silence_fade = |k: Option<&(Piece, usize)>| match k {
        Some((Piece::Silence { .. }, f)) => *f,
        _ => 0,
    };
    let pieces = (0..raw.len())
        .map(|i| match raw[i].0 {
            Piece::Kept { src0, src1, .. } => {
                let prev = if i > 0 { raw.get(i - 1) } else { None };
                let len = src1 - src0;
                let fade_in = gap_ending_at(src0).max(silence_fade(prev)).min(len / 2);
                let fade_out = gap_starting_at(src1)
                    .max(silence_fade(raw.get(i + 1)))
                    .min(len / 2);
                Piece::Kept {
                    src0,
                    src1,
                    fade_in,
                    fade_out,
                }
            }
            p => p,
        })
        .collect();
    Layout { pieces, clip_len }
}

impl Layout {
    /// No edits: the output is the original timeline.
    pub fn is_identity(&self) -> bool {
        matches!(self.pieces.as_slice(),
            [Piece::Kept { src0: 0, src1, fade_in: 0, fade_out: 0 }] if *src1 == self.clip_len)
            || (self.clip_len == 0 && self.pieces.is_empty())
    }

    pub fn output_len(&self) -> usize {
        self.pieces
            .iter()
            .map(|p| match *p {
                Piece::Kept { src0, src1, .. } => src1 - src0,
                Piece::Silence { len, .. } => len,
            })
            .sum()
    }

    /// Assemble the output from a full-length render of the original timeline.
    /// Kept samples outside the fades are copied exactly.
    pub fn apply(&self, audio: &AudioBuffer) -> AudioBuffer {
        let n = self.output_len();
        let mut channels: Vec<Vec<f32>> = (0..audio.num_channels())
            .map(|_| Vec::with_capacity(n))
            .collect();
        for p in &self.pieces {
            match *p {
                Piece::Kept {
                    src0,
                    src1,
                    fade_in,
                    fade_out,
                } => {
                    let len = src1 - src0;
                    for (ch, out) in audio.channels.iter().zip(channels.iter_mut()) {
                        let start = out.len();
                        out.extend_from_slice(&ch[src0..src1]);
                        let seg = &mut out[start..];
                        for k in 0..fade_in {
                            seg[k] = (seg[k] as f64 * ramp(k, fade_in)) as f32;
                        }
                        for k in 0..fade_out {
                            let i = len - 1 - k;
                            seg[i] = (seg[i] as f64 * ramp(k, fade_out)) as f32;
                        }
                    }
                }
                Piece::Silence { len, .. } => {
                    for out in channels.iter_mut() {
                        out.resize(out.len() + len, 0.0);
                    }
                }
            }
        }
        AudioBuffer::new(audio.sample_rate, channels)
    }

    /// Where each removed stretch and each inserted silence shows in the
    /// output, in output order. A removal is marked at its join (before any
    /// silence inserted there).
    pub fn marks(&self) -> Vec<Mark> {
        let mut marks = Vec::new();
        let mut out = 0;
        let mut prev_end: Option<usize> = None;
        let mut join_out = 0;
        for p in &self.pieces {
            match *p {
                Piece::Kept { src0, src1, .. } => {
                    let gap_from = prev_end.unwrap_or(0);
                    if src0 > gap_from {
                        marks.push(Mark::Removed {
                            from: gap_from,
                            to: src0,
                            out: join_out,
                        });
                    }
                    out += src1 - src0;
                    prev_end = Some(src1);
                    join_out = out;
                }
                Piece::Silence { at, len } => {
                    marks.push(Mark::Inserted { at, len, out });
                    out += len;
                }
            }
        }
        let end = prev_end.unwrap_or(0);
        if end < self.clip_len {
            marks.push(Mark::Removed {
                from: end,
                to: self.clip_len,
                out: join_out,
            });
        }
        // At the same point, the removal comes before the silence placed at its join.
        marks.sort_by_key(|m| (m.out(), matches!(m, Mark::Inserted { .. })));
        marks
    }

    /// The marks as records (seconds), for logs and interfaces.
    pub fn marks_json(&self, sample_rate: u32) -> Value {
        let s = |n: usize| round_to(n as f64 / sample_rate as f64, 6);
        Value::Array(
            self.marks()
                .iter()
                .map(|m| match *m {
                    Mark::Removed { from, to, out } => json!({
                        "kind": "removed", "from_s": s(from), "to_s": s(to),
                        "duration_s": s(to - from), "at_output_s": s(out),
                    }),
                    Mark::Inserted { at, len, out } => json!({
                        "kind": "inserted", "at_s": s(at), "duration_s": s(len),
                        "at_output_s": s(out),
                    }),
                })
                .collect(),
        )
    }

    /// Cue markers for an exported file: output sample and a plain-ASCII label.
    pub fn cues(&self, sample_rate: u32) -> Vec<(u32, String)> {
        let s = |n: usize| n as f64 / sample_rate as f64;
        self.marks()
            .iter()
            .map(|m| match *m {
                Mark::Removed { from, to, out } => (
                    out as u32,
                    format!(
                        "removed {:.3}-{:.3} s of the original ({:.3} s)",
                        s(from),
                        s(to),
                        s(to - from)
                    ),
                ),
                Mark::Inserted { at, len, out } => (
                    out as u32,
                    format!(
                        "inserted {:.3} s of silence at {:.3} s of the original",
                        s(len),
                        s(at)
                    ),
                ),
            })
            .collect()
    }

    /// The pieces in seconds, with where each lands in the output: what an
    /// interface needs to play the output and show the original's timeline.
    pub fn map_json(&self, sample_rate: u32) -> Value {
        let s = |n: usize| round_to(n as f64 / sample_rate as f64, 6);
        let mut out = 0;
        let pieces: Vec<Value> = self
            .pieces
            .iter()
            .map(|p| {
                let v = match *p {
                    Piece::Kept { src0, src1, .. } => {
                        json!({ "kind": "kept", "src0": s(src0), "src1": s(src1), "out0": s(out) })
                    }
                    Piece::Silence { at, len } => {
                        json!({ "kind": "silence", "at": s(at), "len": s(len), "out0": s(out) })
                    }
                };
                out += match *p {
                    Piece::Kept { src0, src1, .. } => src1 - src0,
                    Piece::Silence { len, .. } => len,
                };
                v
            })
            .collect();
        json!({
            "edited": !self.is_identity(),
            "duration_s": s(self.output_len()),
            "original_duration_s": s(self.clip_len),
            "pieces": pieces,
            "marks": self.marks_json(sample_rate),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rem(s0: usize, s1: usize, fade: usize) -> Edit {
        Edit::Remove { s0, s1, fade }
    }
    fn ins(at: usize, len: usize, fade: usize) -> Edit {
        Edit::Insert { at, len, fade }
    }
    fn kept(p: &Piece) -> (usize, usize) {
        match *p {
            Piece::Kept { src0, src1, .. } => (src0, src1),
            _ => panic!("not kept: {p:?}"),
        }
    }

    #[test]
    fn no_edits_is_the_original() {
        let l = layout(&[], 100);
        assert!(l.is_identity());
        let a = AudioBuffer::mono(10, (0..100).map(|i| i as f32).collect());
        assert_eq!(l.apply(&a), a);
        assert!(l.marks().is_empty());
    }

    #[test]
    fn removals_merge_and_the_rest_is_copied_exactly() {
        let l = layout(&[rem(20, 40, 0), rem(30, 50, 0), rem(80, 100, 0)], 100);
        assert_eq!(l.pieces.len(), 2);
        assert_eq!(kept(&l.pieces[0]), (0, 20));
        assert_eq!(kept(&l.pieces[1]), (50, 80));
        let a = AudioBuffer::mono(10, (0..100).map(|i| i as f32).collect());
        let out = l.apply(&a);
        let want: Vec<f32> = (0..20).chain(50..80).map(|i| i as f32).collect();
        assert_eq!(out.channels[0], want);
        assert_eq!(
            l.marks(),
            vec![
                Mark::Removed {
                    from: 20,
                    to: 50,
                    out: 20
                },
                Mark::Removed {
                    from: 80,
                    to: 100,
                    out: 50
                },
            ]
        );
    }

    #[test]
    fn silence_goes_in_at_its_point_and_inside_a_removal_at_the_join() {
        let l = layout(
            &[ins(10, 5, 0), rem(40, 60, 0), ins(50, 3, 0), ins(100, 2, 0)],
            100,
        );
        let a = AudioBuffer::mono(10, vec![1.0; 100]);
        let out = l.apply(&a);
        assert_eq!(out.len(), 100 + 5 - 20 + 3 + 2);
        assert_eq!(&out.channels[0][10..15], &[0.0; 5]);
        assert_eq!(out.channels[0][15], 1.0);
        let marks = l.marks();
        assert_eq!(
            marks[0],
            Mark::Inserted {
                at: 10,
                len: 5,
                out: 10
            }
        );
        // The removal is marked at its join, the silence moved there follows it.
        assert_eq!(
            marks[1],
            Mark::Removed {
                from: 40,
                to: 60,
                out: 45
            }
        );
        assert_eq!(
            marks[2],
            Mark::Inserted {
                at: 40,
                len: 3,
                out: 45
            }
        );
        assert_eq!(
            marks[3],
            Mark::Inserted {
                at: 100,
                len: 2,
                out: 88
            }
        );
    }

    #[test]
    fn silences_at_the_same_point_keep_their_order_and_the_order_of_edits_does_not_matter() {
        let a = layout(&[ins(10, 1, 0), rem(20, 30, 4), ins(10, 2, 0)], 100);
        let b = layout(&[rem(20, 30, 4), ins(10, 1, 0), ins(10, 2, 0)], 100);
        assert_eq!(a, b);
        let lens: Vec<usize> = a
            .pieces
            .iter()
            .filter_map(|p| match *p {
                Piece::Silence { len, .. } => Some(len),
                _ => None,
            })
            .collect();
        assert_eq!(lens, vec![1, 2]);
    }

    #[test]
    fn joins_fade_out_and_in_and_only_there() {
        let l = layout(&[rem(40, 60, 8)], 100);
        let a = AudioBuffer::mono(10, vec![1.0; 100]);
        let out = l.apply(&a).channels[0].clone();
        assert_eq!(out.len(), 80);
        // Untouched away from the join.
        assert!(out[..32].iter().all(|&x| x == 1.0));
        assert!(out[48..].iter().all(|&x| x == 1.0));
        // Down to near zero at the join from both sides, symmetrically.
        assert!(out[39] < 0.05 && out[40] < 0.05);
        for k in 0..8 {
            assert_eq!(out[39 - k], out[40 + k]);
        }
        for k in 32..39 {
            assert!(out[k] > out[k + 1], "fading out at {k}");
        }
        // The ends of the recording are not faded when no edit made them.
        assert_eq!(out[0], 1.0);
        assert_eq!(out[79], 1.0);
        // A removal at the very start fades the new start in.
        let s = layout(&[rem(0, 10, 4)], 100).apply(&a).channels[0].clone();
        assert!(s[0] < 0.2 && s[4] == 1.0);
    }

    #[test]
    fn cue_labels_say_what_happened_in_the_original_time() {
        let l = layout(&[rem(48_000, 96_000, 0), ins(144_000, 24_000, 0)], 192_000);
        assert_eq!(
            l.cues(48_000),
            vec![
                (
                    48_000,
                    "removed 1.000-2.000 s of the original (1.000 s)".to_string()
                ),
                (
                    96_000,
                    "inserted 0.500 s of silence at 3.000 s of the original".to_string()
                ),
            ]
        );
        let m = l.map_json(48_000);
        assert_eq!(m["edited"], true);
        assert_eq!(m["duration_s"], 3.5);
        assert_eq!(m["pieces"][2]["kind"], "silence");
        assert_eq!(m["pieces"][2]["out0"], 2.0);
        assert_eq!(m["pieces"][3]["out0"], 2.5);
    }

    #[test]
    fn removing_everything_leaves_an_empty_output() {
        let l = layout(&[rem(0, 100, 5)], 100);
        assert_eq!(l.output_len(), 0);
        assert_eq!(l.apply(&AudioBuffer::mono(10, vec![1.0; 100])).len(), 0);
        assert_eq!(
            l.marks(),
            vec![Mark::Removed {
                from: 0,
                to: 100,
                out: 0
            }]
        );
    }
}
