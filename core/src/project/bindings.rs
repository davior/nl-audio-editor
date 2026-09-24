//! Bindings: the adaptive form of a step's values.
//!
//! `infer` runs when a step is accepted and ties its hand-set values to the
//! clip's analysis — a cut around 3,100–3,200 Hz becomes "the band around the
//! detected line near 3,150 Hz"; a +29.4 dB gain becomes "bring the peak to
//! −1 dBFS". `apply` re-derives those values on another clip. This is what
//! makes manual work reusable without anyone explaining it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::step::{Binding, BindingSource, Step};
use crate::analysis::{Features, TonalLine};
use crate::audio::AudioBuffer;
use crate::math::round_to;
use crate::scope::Scope;

/// A line is "the same line" on another clip if within this ratio of the
/// recorded frequency (±30 %); otherwise the line of the same prominence rank.
pub const NEAR_RATIO: f64 = 1.3;

fn nearest_line(lines: &[TonalLine], lo: f64, hi: f64) -> Option<(usize, &TonalLine)> {
    let centre = (lo + hi) / 2.0;
    let slack = (hi - lo).max(30.0);
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.freq_hz >= lo - slack && l.freq_hz <= hi + slack)
        .min_by(|a, b| {
            (a.1.freq_hz - centre)
                .abs()
                .total_cmp(&(b.1.freq_hz - centre).abs())
        })
}

fn line_binding(line: &TonalLine, rank: usize, field: &str, recorded: f64, note: &str) -> Binding {
    let base = if field == "prominence_db" {
        line.prominence_db
    } else {
        line.freq_hz
    };
    Binding {
        feature: if field == "prominence_db" {
            "tonal_lines.prominence_db".into()
        } else {
            "tonal_lines".into()
        },
        select: Some(json!({ "near_hz": line.freq_hz, "rank": rank + 1 })),
        relation: "offset".into(),
        value: Some(round_to(recorded - base, 3)),
        source: BindingSource::Inferred,
        note: note.into(),
    }
}

fn ac_level_db(input: &AudioBuffer, t0: f64, t1: f64) -> f64 {
    let (a, b) = (input.time_to_sample(t0), input.time_to_sample(t1));
    if b <= a {
        return crate::math::DB_FLOOR;
    }
    let mut acc = 0.0;
    for ch in &input.channels {
        let x = &ch[a..b];
        let mean = x.iter().map(|&v| v as f64).sum::<f64>() / x.len() as f64;
        acc += x
            .iter()
            .map(|&v| (v as f64 - mean) * (v as f64 - mean))
            .sum::<f64>();
    }
    crate::math::power_to_db(acc / ((b - a) * input.num_channels()) as f64)
}

/// Bindings for a step's hand-set values, from the features of its input.
/// Values that are `auto` need none: they are re-measured on every clip.
pub fn infer(step: &Step, before: &Features, input: &AudioBuffer) -> BTreeMap<String, Binding> {
    let mut out = BTreeMap::new();
    let p = &step.params;
    let duration = before.t1 - before.t0;
    let lines = &before.tonal_lines;
    match step.op.as_str() {
        "gain" if step.scope == Scope::Clip => {
            if let Some(g) = p["gain_db"].as_f64() {
                out.insert(
                    "gain_db".into(),
                    Binding {
                        feature: "peak_dbfs".into(),
                        select: None,
                        relation: "target".into(),
                        value: Some(round_to(before.peak_dbfs + g, 2)),
                        source: BindingSource::Inferred,
                        note: format!("brings the peak to {:.1} dBFS", before.peak_dbfs + g),
                    },
                );
            }
        }
        "band_cut" => {
            if let (Some(lo), Some(hi)) = (p["f_lo"].as_f64(), p["f_hi"].as_f64()) {
                if let Some((rank, l)) = nearest_line(lines, lo, hi) {
                    let note = format!("the band around the detected line at {:.1} Hz", l.freq_hz);
                    out.insert("f_lo".into(), line_binding(l, rank, "freq_hz", lo, &note));
                    out.insert("f_hi".into(), line_binding(l, rank, "freq_hz", hi, &note));
                    if let Some(d) = p["depth_db"].as_f64() {
                        out.insert(
                            "depth_db".into(),
                            line_binding(
                                l,
                                rank,
                                "prominence_db",
                                d,
                                "the cut relative to the line's prominence",
                            ),
                        );
                    }
                }
            }
        }
        "noise_reduce" => {
            if let (Some(t0), Some(t1)) = (p["profile"]["t0"].as_f64(), p["profile"]["t1"].as_f64())
            {
                if let Some(q) = &before.quietest_region {
                    if ac_level_db(input, t0, t1) <= q.level_dbfs + 6.0 {
                        out.insert(
                            "profile".into(),
                            Binding {
                                feature: "quietest_region".into(),
                                select: None,
                                relation: "equals".into(),
                                value: None,
                                source: BindingSource::Inferred,
                                note: "a quiet part of the clip: the quietest steady region on another clip".into(),
                            },
                        );
                    }
                }
            }
        }
        "line_reduce" => {
            if let Some(items) = p["lines"].as_array() {
                for (i, it) in items.iter().enumerate() {
                    let f = it["freq_hz"].as_f64().unwrap_or(0.0);
                    let w = it["width_hz"].as_f64().unwrap_or(0.0);
                    if let Some((rank, l)) = nearest_line(lines, f - w / 2.0, f + w / 2.0) {
                        let note = format!("the detected line at {:.1} Hz", l.freq_hz);
                        out.insert(
                            format!("lines[{i}].freq_hz"),
                            line_binding(l, rank, "freq_hz", f, &note),
                        );
                        if let Some(d) = it["depth_db"].as_f64() {
                            out.insert(
                                format!("lines[{i}].depth_db"),
                                line_binding(l, rank, "prominence_db", d, &note),
                            );
                        }
                    }
                }
            }
        }
        "compressor" => {
            if let (Some(t), Some(lufs)) = (p["threshold_dbfs"].as_f64(), before.loudness_lufs) {
                out.insert(
                    "threshold_dbfs".into(),
                    Binding {
                        feature: "loudness_lufs".into(),
                        select: None,
                        relation: "offset".into(),
                        value: Some(round_to(t - lufs, 2)),
                        source: BindingSource::Inferred,
                        note: "threshold relative to the programme loudness".into(),
                    },
                );
            }
        }
        _ => {}
    }
    // Scope.
    if let Some((t0, t1)) = step.scope.time() {
        if duration > 0.0 && (t1 - t0) >= 0.95 * duration {
            out.insert(
                "scope.time".into(),
                Binding {
                    feature: "clip".into(),
                    select: None,
                    relation: "whole".into(),
                    value: None,
                    source: BindingSource::Inferred,
                    note: "the whole clip".into(),
                },
            );
        } else if duration > 0.0 {
            for (k, t) in [("scope.t0", t0), ("scope.t1", t1)] {
                out.insert(
                    k.into(),
                    Binding {
                        feature: "clip".into(),
                        select: None,
                        relation: "fraction".into(),
                        value: Some(round_to(t / duration, 6)),
                        source: BindingSource::Inferred,
                        note: "position in the clip (a weak guess on another recording)".into(),
                    },
                );
            }
        }
    }
    if let Some((lo, hi)) = step.scope.band() {
        if let Some((rank, l)) = nearest_line(lines, lo, hi) {
            let note = format!("the band around the detected line at {:.1} Hz", l.freq_hz);
            out.insert(
                "scope.f_lo".into(),
                line_binding(l, rank, "freq_hz", lo, &note),
            );
            out.insert(
                "scope.f_hi".into(),
                line_binding(l, rank, "freq_hz", hi, &note),
            );
        }
    }
    // Declared bindings take precedence over inferred ones.
    for (k, b) in &step.bindings {
        out.insert(k.clone(), b.clone());
    }
    out
}

/// One value changed (or kept) by an adaptive replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DiffEntry {
    pub step: usize,
    pub op: String,
    pub param: String,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub recorded: Value,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub new: Value,
    pub reason: String,
}

fn find_line<'a>(lines: &'a [TonalLine], select: &Option<Value>) -> Option<&'a TonalLine> {
    let sel = select.as_ref()?;
    if let Some(near) = sel["near_hz"].as_f64() {
        let best = lines
            .iter()
            .filter(|l| l.freq_hz >= near / NEAR_RATIO && l.freq_hz <= near * NEAR_RATIO)
            .min_by(|a, b| log_distance(a.freq_hz, near).total_cmp(&log_distance(b.freq_hz, near)));
        if best.is_some() {
            return best;
        }
    }
    let rank = sel["rank"].as_u64()? as usize;
    lines.get(rank.checked_sub(1)?)
}

fn log_distance(f: f64, near: f64) -> f64 {
    crate::math::ln(f / near).abs()
}

fn get_path(params: &Value, scope: &Scope, key: &str) -> Value {
    match key {
        "scope.t0" => json!(scope.time().map(|t| t.0)),
        "scope.t1" => json!(scope.time().map(|t| t.1)),
        "scope.f_lo" => json!(scope.band().map(|b| b.0)),
        "scope.f_hi" => json!(scope.band().map(|b| b.1)),
        "scope.time" => json!(scope.time().map(|t| [t.0, t.1])),
        k if k.starts_with("lines[") => {
            let (i, field) = parse_line_key(k);
            params["lines"][i][field].clone()
        }
        k => params[k].clone(),
    }
}

fn parse_line_key(k: &str) -> (usize, &str) {
    let i = k[6..k.find(']').unwrap_or(6)].parse().unwrap_or(0);
    let field = k.rsplit('.').next().unwrap_or("");
    (i, field)
}

fn set_scope_time(scope: &mut Scope, t0: Option<f64>, t1: Option<f64>) {
    match scope {
        Scope::TimeRange { t0: a, t1: b } | Scope::TfPatch { t0: a, t1: b, .. } => {
            if let Some(v) = t0 {
                *a = v;
            }
            if let Some(v) = t1 {
                *b = v;
            }
        }
        _ => {}
    }
}

fn set_scope_band(scope: &mut Scope, lo: Option<f64>, hi: Option<f64>) {
    match scope {
        Scope::Band { f_lo, f_hi } | Scope::TfPatch { f_lo, f_hi, .. } => {
            if let Some(v) = lo {
                *f_lo = v;
            }
            if let Some(v) = hi {
                *f_hi = v;
            }
        }
        _ => {}
    }
}

/// Re-derive bound values on another clip. Returns the diff (every bound value,
/// changed or not, with the reason).
pub fn apply(
    step_index: usize,
    op: &str,
    bindings: &BTreeMap<String, Binding>,
    params: &mut Value,
    scope: &mut Scope,
    features: &Features,
) -> Vec<DiffEntry> {
    let duration = features.t1 - features.t0;
    let mut diff = Vec::new();
    for (key, b) in bindings {
        let recorded = get_path(params, scope, key);
        let mut reason = b.note.clone();
        let new: Option<Value> = match (b.feature.as_str(), b.relation.as_str()) {
            ("tonal_lines", "offset") | ("tonal_lines.prominence_db", "offset") => {
                match find_line(&features.tonal_lines, &b.select) {
                    Some(l) => {
                        let base = if b.feature == "tonal_lines" {
                            l.freq_hz
                        } else {
                            l.prominence_db
                        };
                        let mut v = round_to(base + b.value.unwrap_or(0.0), 2);
                        if b.feature != "tonal_lines" {
                            v = v.clamp(0.0, 60.0);
                        }
                        reason = format!(
                            "line at {:.1} Hz (prominence {:.1} dB) on this clip",
                            l.freq_hz, l.prominence_db
                        );
                        Some(json!(v))
                    }
                    None => {
                        reason = "no matching line on this clip; recorded value kept".into();
                        None
                    }
                }
            }
            ("peak_dbfs", "target") => {
                let target = b.value.unwrap_or(-1.0);
                reason = format!(
                    "peak {:.2} dBFS here → target {:.1} dBFS",
                    features.peak_dbfs, target
                );
                Some(json!(round_to(
                    (target - features.peak_dbfs).clamp(-60.0, 60.0),
                    3
                )))
            }
            ("loudness_lufs", "offset") => features.loudness_lufs.map(|l| {
                reason = format!("loudness {l:.1} LUFS here");
                json!(round_to((l + b.value.unwrap_or(0.0)).clamp(-80.0, 0.0), 2))
            }),
            ("quietest_region", "equals") => match &features.quietest_region {
                Some(q) => {
                    reason = format!("quietest steady region here: {:.2}–{:.2} s", q.t0, q.t1);
                    Some(json!({ "t0": q.t0, "t1": q.t1 }))
                }
                None => None,
            },
            ("clip", "whole") => {
                reason = "the whole clip".into();
                Some(json!([features.t0, features.t1]))
            }
            ("clip", "fraction") => Some(json!(round_to(b.value.unwrap_or(0.0) * duration, 3))),
            _ => None,
        };
        if let Some(v) = &new {
            match key.as_str() {
                "scope.t0" => set_scope_time(scope, v.as_f64(), None),
                "scope.t1" => set_scope_time(scope, None, v.as_f64()),
                "scope.time" => set_scope_time(scope, v[0].as_f64(), v[1].as_f64()),
                "scope.f_lo" => set_scope_band(scope, v.as_f64(), None),
                "scope.f_hi" => set_scope_band(scope, None, v.as_f64()),
                k if k.starts_with("lines[") => {
                    let (i, field) = parse_line_key(k);
                    params["lines"][i][field] = v.clone();
                }
                k => params[k] = v.clone(),
            }
        }
        diff.push(DiffEntry {
            step: step_index,
            op: op.to_string(),
            param: key.clone(),
            recorded,
            new: new.unwrap_or_else(|| get_path(params, scope, key)),
            reason,
        });
    }
    diff
}
