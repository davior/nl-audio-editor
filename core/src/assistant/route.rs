//! Routine requests answered without the model: the built-in clean-up, undo,
//! what to listen to, and commands that state their own numbers. The same
//! words always do the same thing, cost nothing and need no network.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::scope::Scope;

/// What the interface has selected, if anything.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Selection {
    /// A time range: `[t0, t1]` in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<[f64; 2]>,
    /// A time × frequency area: `[t0, t1, f_lo, f_hi]` (seconds, hertz).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<[f64; 4]>,
}

impl Selection {
    /// The area if there is one, else the time range.
    pub fn scope(&self) -> Option<Scope> {
        if let Some([t0, t1, f_lo, f_hi]) = self.area {
            return Some(Scope::TfPatch { t0, t1, f_lo, f_hi });
        }
        self.time_scope()
    }

    /// The time range, or the time span of the area.
    pub fn time_scope(&self) -> Option<Scope> {
        match (self.time, self.area) {
            (Some([t0, t1]), _) => Some(Scope::TimeRange { t0, t1 }),
            (None, Some([t0, t1, _, _])) => Some(Scope::TimeRange { t0, t1 }),
            _ => None,
        }
    }
}

/// How a request is handled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "route", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Route {
    /// Replay a built-in recipe, adaptively.
    Recipe { name: String },
    /// Steps whose values the words give.
    Steps { steps: Vec<RoutedStep> },
    /// Switch what is heard and drawn: `source`, `stack` or `residual`.
    Listen { which: String },
    /// Take back the last change to the stack (the history is kept).
    Undo,
    /// Repeat the last change taken back.
    Redo,
    /// Needs the model.
    Model,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RoutedStep {
    pub op: String,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    pub scope: Scope,
    /// What was understood, in words (shown, and recorded as the rationale).
    pub understood: String,
}

impl RoutedStep {
    /// A draft for a preview: the user's own request, with what was understood
    /// recorded as the rationale.
    pub fn draft(
        &self,
        origin: crate::project::step::Origin,
        words: &str,
    ) -> crate::project::step::StepDraft {
        let mut d = crate::project::step::StepDraft::new(
            &self.op,
            self.params.clone(),
            self.scope.clone(),
            crate::provenance::Actor::user(),
            origin,
        );
        d.intent = Some(words.to_string());
        d.rationale = Some(self.understood.clone());
        d
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Hz,
    Db,
    /// Integrated loudness (LUFS, LKFS or LU).
    Lufs,
    /// Time, scaled to seconds.
    Seconds,
    None,
}

/// "1:20" → "80 s", "1:02:03.5" → "3723.5 s": clock times become seconds.
fn clock_times(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let starts = chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric());
        if starts {
            // Parts separated by ':' with two digits after each colon.
            let mut j = i;
            let mut parts: Vec<String> = Vec::new();
            let mut cur = String::new();
            while j < chars.len() && chars[j].is_ascii_digit() {
                cur.push(chars[j]);
                j += 1;
            }
            parts.push(cur);
            while chars.get(j) == Some(&':')
                && chars.get(j + 1).is_some_and(|c| c.is_ascii_digit())
                && chars.get(j + 2).is_some_and(|c| c.is_ascii_digit())
                && !chars.get(j + 3).is_some_and(|c| c.is_ascii_digit())
            {
                parts.push(chars[j + 1..j + 3].iter().collect());
                j += 3;
            }
            if parts.len() >= 2 && parts.len() <= 3 {
                let mut frac = String::new();
                if chars.get(j) == Some(&'.')
                    && chars.get(j + 1).is_some_and(|c| c.is_ascii_digit())
                {
                    frac.push('.');
                    j += 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        frac.push(chars[j]);
                        j += 1;
                    }
                }
                let total = parts
                    .iter()
                    .fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0))
                    + format!("0{frac}").parse::<f64>().unwrap_or(0.0);
                out.push_str(&format!("{total} s"));
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Lower case, dashes as "to", thousands separators removed, spacing
/// collapsed; spoken forms ("minus 1", "1 point 5", "kilohertz") as they
/// would be typed.
fn normalise(words: &str) -> String {
    let lower = clock_times(&words.to_lowercase())
        .replace(['–', '—'], " to ")
        .replace('−', "-")
        .replace("dbfs", "db")
        .replace("decibels", "db")
        .replace("decibel", "db")
        .replace("kilohertz", "khz")
        .replace("hertz", "hz");
    let chars: Vec<char> = lower.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        let digit = |j: usize| chars.get(j).is_some_and(|c| c.is_ascii_digit());
        // "3,100" → "3100"
        if c == ',' && i > 0 && digit(i - 1) && digit(i + 1) && digit(i + 2) && digit(i + 3) {
            continue;
        }
        // "3100-3200" → "3100 to 3200"
        if c == '-' && i > 0 && digit(i - 1) && digit(i + 1) {
            out.push_str(" to ");
            continue;
        }
        out.push(c);
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let number = |w: &str| w.starts_with(|c: char| c.is_ascii_digit());
    let integer = |w: &str| {
        let d = w.strip_prefix('-').unwrap_or(w);
        !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())
    };
    let mut said: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let next = words.get(i + 1).copied();
        match words[i] {
            // "minus 1 db" → "-1 db"
            "minus" | "negative" if next.is_some_and(number) => {
                said.push(format!("-{}", next.unwrap_or_default()));
                i += 2;
            }
            // "1 point 5 seconds" → "1.5 seconds"
            "point"
                if said.last().is_some_and(|w| integer(w))
                    && next.is_some_and(|w| w.chars().all(|c| c.is_ascii_digit())) =>
            {
                let whole = said.pop().unwrap_or_default();
                said.push(format!("{whole}.{}", next.unwrap_or_default()));
                i += 2;
            }
            w => {
                said.push(w.to_string());
                i += 1;
            }
        }
    }
    said.join(" ").trim_end_matches(['.', '!', '?']).to_string()
}

/// Numbers in order, each with the unit written after it and that unit's
/// scale (1000 for kilohertz).
fn numbers(text: &str) -> Vec<(f64, Unit, f64)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts = c.is_ascii_digit()
            || ((c == '-' || c == '+' || c == '.')
                && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
                && (i == 0 || !chars[i - 1].is_alphanumeric()));
        if !starts {
            i += 1;
            continue;
        }
        let begin = i;
        i += 1;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        let Ok(v) = chars[begin..i].iter().collect::<String>().parse::<f64>() else {
            continue;
        };
        let rest: String = chars[i..].iter().collect();
        let rest = rest.trim_start();
        // A unit word must end there: "2 s" and "2 seconds", not "2 steps".
        let word = |w: &str| {
            rest.starts_with(w) && !rest[w.len()..].starts_with(|c: char| c.is_alphabetic())
        };
        let (unit, scale) = if rest.starts_with("khz") || rest.starts_with("k ") || rest == "k" {
            (Unit::Hz, 1000.0)
        } else if rest.starts_with("hz") {
            (Unit::Hz, 1.0)
        } else if rest.starts_with("db") {
            (Unit::Db, 1.0)
        } else if ["lufs", "lkfs", "lu"].iter().any(|w| word(w)) {
            (Unit::Lufs, 1.0)
        } else if ["ms", "msec", "millisecond", "milliseconds"]
            .iter()
            .any(|w| word(w))
        {
            (Unit::Seconds, 0.001)
        } else if ["s", "sec", "secs", "second", "seconds"]
            .iter()
            .any(|w| word(w))
        {
            (Unit::Seconds, 1.0)
        } else if ["min", "mins", "minute", "minutes"].iter().any(|w| word(w)) {
            (Unit::Seconds, 60.0)
        } else {
            (Unit::None, 1.0)
        };
        out.push((v, unit, scale));
    }
    out
}

fn has_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// Whether `word` appears in `text` as a whole word ("gate", not "investigate").
fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Frequencies in hertz: numbers marked as frequencies, and unmarked numbers
/// in a range written "X to Y kHz" (the unit written once applies to both).
fn frequencies(nums: &[(f64, Unit, f64)]) -> Vec<f64> {
    let mut f = Vec::new();
    for (i, &(v, u, scale)) in nums.iter().enumerate() {
        match (u, nums.get(i + 1)) {
            (Unit::Hz, _) => f.push(v * scale),
            (Unit::None, Some(&(_, Unit::Hz, next_scale))) => f.push(v * next_scale),
            _ => {}
        }
    }
    f
}

/// Times in seconds: numbers marked as times, and unmarked numbers in a range
/// written "X to Y seconds" (the unit written once applies to both).
fn seconds(nums: &[(f64, Unit, f64)]) -> Vec<f64> {
    let mut f = Vec::new();
    for (i, &(v, u, scale)) in nums.iter().enumerate() {
        match (u, nums.get(i + 1)) {
            (Unit::Seconds, _) => f.push(v * scale),
            (Unit::None, Some(&(_, Unit::Seconds, next_scale))) => f.push(v * next_scale),
            _ => {}
        }
    }
    f
}

/// Words naming something to process rather than a stretch of time.
const TARGETS: &[&str] = &[
    "hum", "noise", "hiss", "line", "whine", "tone", "whistle", "click", "bang", "knock", "thump",
    "peak", "band", "frequen", "hz", "dc", "bass", "treble", "rumble", "voice", "speech", "echo",
    "reverb", "level", "loud",
];

/// A removal or an inserted silence, if the words ask for one.
fn time_edit(t: &str, nums: &[(f64, Unit, f64)], selection: Option<&Selection>) -> Option<Route> {
    let secs = seconds(nums);
    let removing = has_any(
        t,
        &[
            "remove",
            "cut",
            "delete",
            "drop",
            "take out",
            "trim",
            "edit out",
            "get rid of",
        ],
    );
    let inserting = has_any(t, &["insert", "add", "put in", "pad"])
        && has_any(t, &["silence", "gap", "pause", "blank"]);
    let this_part = has_any(
        t,
        &[
            " here",
            "this part",
            "this bit",
            "this section",
            "this stretch",
            "selection",
            "selected",
        ],
    );
    let names_a_target = has_any(t, TARGETS);
    let removal = |t0: f64, t1: f64| {
        let (t0, t1) = (t0.min(t1), t0.max(t1));
        let scope = Scope::TimeRange { t0, t1 };
        step(
            "remove_time",
            json!({}),
            scope,
            format!(
                "Remove {t0:.3}–{t1:.3} s of the original from the output ({:.3} s); processing still covers it.",
                t1 - t0
            ),
        )
    };
    if removing && !inserting {
        if secs.len() >= 2 && (t.contains(" to ") || t.contains(" and ")) {
            return Some(removal(secs[0], secs[1]));
        }
        if secs.len() == 1 && has_any(t, &["first "]) {
            return Some(removal(0.0, secs[0]));
        }
        if secs.is_empty() && this_part && !names_a_target {
            if let Some(Scope::TimeRange { t0, t1 }) = selection.and_then(Selection::time_scope) {
                return Some(removal(t0, t1));
            }
        }
        return None;
    }
    if inserting {
        // "insert 2 s of silence at 30 s": the length before "at", the point after it.
        let (len_part, at_part) = match t.find(" at ") {
            Some(i) => (&t[..i], Some(&t[i + 4..])),
            None => (t, None),
        };
        let len = seconds(&numbers(len_part)).first().copied()?;
        let at = match at_part {
            Some(a) if has_any(a, &["start", "beginning"]) => Some(0.0),
            Some(a) => numbers(a).first().map(
                |&(v, u, scale)| {
                    if u == Unit::Seconds {
                        v * scale
                    } else {
                        v
                    }
                },
            ),
            None if has_any(t, &["start", "beginning"]) => Some(0.0),
            None if this_part => selection
                .and_then(Selection::time_scope)
                .and_then(|s| s.time())
                .map(|(t0, _)| t0),
            None => None,
        }?;
        return Some(step(
            "insert_silence",
            json!({ "at_s": at, "duration_s": len }),
            Scope::Clip,
            format!("Insert {len:.3} s of silence at {at:.3} s of the original."),
        ));
    }
    None
}

fn decibels(nums: &[(f64, Unit, f64)]) -> Vec<f64> {
    nums.iter()
        .filter(|(_, u, _)| *u == Unit::Db)
        .map(|(v, _, _)| *v)
        .collect()
}

/// The filters, EQ, gate, loudness and hum removal, when the words name one
/// with what it needs. Checked before the generic cuts, so "cut below 100 Hz"
/// is a high-pass and not a line at 100 Hz.
fn catalogue(
    t: &str,
    nums: &[(f64, Unit, f64)],
    freqs: &[f64],
    dbs: &[f64],
    span: Option<Scope>,
) -> Option<Route> {
    let scope = span.unwrap_or(Scope::Clip);
    let whole = scope_words(&scope);
    let cutting = has_any(
        t,
        &[
            "cut",
            "remove",
            "reduce",
            "filter",
            "roll off",
            "get rid of",
            "take out",
            "lower",
            "kill",
            "tame",
        ],
    );
    let boosting = has_any(t, &["boost", "raise", "lift", "bring up"]);

    // Loudness: "normalise to -16 LUFS", "loudness to -23".
    let lufs = nums.iter().find(|(_, u, _)| *u == Unit::Lufs).map(|n| n.0);
    let loudness = has_any(t, &["loudness", "lufs", "lkfs"]);
    if lufs.is_some() || (loudness && has_any(t, &["normalise", "normalize", " to ", "target"])) {
        let target = lufs.or_else(|| nums.iter().find(|(_, u, _)| *u == Unit::None).map(|n| n.0));
        let target = target.map(|v| -v.abs());
        let (params, words) = match target {
            Some(v) => (json!({ "target_lufs": v }), format!("{v} LUFS")),
            None => (json!({}), "the default −23 LUFS (EBU R128)".to_string()),
        };
        return Some(step(
            "loudness_normalise",
            params,
            scope,
            format!("Bring the integrated loudness to {words}, {whole}."),
        ));
    }

    // High-pass and low-pass: "high-pass at 80 Hz", "cut below 100 Hz", "remove the rumble".
    // "Below" or "above" a frequency means a filter only without an amount:
    // "cut the treble above 5 kHz by 4 dB" is a shelf.
    let one = freqs.len() == 1;
    let beyond = |w: &str| one && cutting && dbs.is_empty() && has_word(t, w);
    let high_pass = has_any(
        t,
        &["high-pass", "high pass", "highpass", "low-cut", "low cut"],
    ) || has_word(t, "hpf")
        || beyond("below")
        || beyond("under")
        || (cutting && t.contains("rumble"));
    if high_pass && freqs.len() <= 1 {
        let (params, words) = match freqs.first() {
            Some(f) => (json!({ "cutoff_hz": f }), format!("{f:.0} Hz")),
            None => (json!({}), "80 Hz".to_string()),
        };
        return Some(step(
            "high_pass",
            params,
            scope,
            format!("Cut below {words} (24 dB per octave, zero phase), {whole}."),
        ));
    }
    let low_pass = has_any(
        t,
        &["low-pass", "low pass", "lowpass", "high-cut", "high cut"],
    ) || has_word(t, "lpf")
        || beyond("above")
        || beyond("over");
    if low_pass && one {
        let f = freqs[0];
        return Some(step(
            "low_pass",
            json!({ "cutoff_hz": f }),
            scope,
            format!("Cut above {f:.0} Hz (24 dB per octave, zero phase), {whole}."),
        ));
    }

    // Shelves: "boost the bass by 3 dB", "cut the treble by 4 dB".
    let bass = has_any(t, &["bass", "low end", "the lows", "low frequencies"]);
    let treble = has_any(t, &["treble", "top end", "the highs", "high frequencies"]);
    if bass != treble && !dbs.is_empty() && freqs.len() <= 1 {
        let d = dbs[0];
        let up = boosting || has_any(t, &["more", "louder"]);
        let down = cutting || has_any(t, &["less", "quieter"]);
        let g = if down && !up {
            -d.abs()
        } else if up && !down {
            d.abs()
        } else {
            d
        };
        let (kind, corner) = if bass {
            ("low", freqs.first().copied().unwrap_or(200.0))
        } else {
            ("high", freqs.first().copied().unwrap_or(4000.0))
        };
        return Some(step(
            "shelf",
            json!({ "kind": kind, "freq_hz": corner, "gain_db": g }),
            scope,
            format!(
                "{} everything {} {corner:.0} Hz by {:.1} dB (a {kind} shelf), {whole}.",
                if g >= 0.0 { "Raise" } else { "Lower" },
                if bass { "below" } else { "above" },
                g.abs()
            ),
        ));
    }

    // A bell: "boost 3 kHz by 3 dB", "dip 250 Hz by 4 dB".
    let dipping = has_word(t, "dip");
    if (boosting || dipping || has_word(t, "bell") || has_word(t, "eq")) && one && !dbs.is_empty() {
        let g = if dipping {
            -dbs[0].abs()
        } else if boosting {
            dbs[0].abs()
        } else {
            dbs[0]
        };
        let f = freqs[0];
        return Some(step(
            "bell",
            json!({ "freq_hz": f, "gain_db": g }),
            scope,
            format!(
                "{} {f:.0} Hz by {:.1} dB, a bell an octave wide, {whole}.",
                if g >= 0.0 { "Raise" } else { "Lower" },
                g.abs()
            ),
        ));
    }

    // Tilt: "brighter by 1.5 dB per octave".
    if has_any(t, &["per octave", "/octave", "an octave"])
        && !dbs.is_empty()
        && has_any(t, &["brighter", "darker", "tilt"])
    {
        let d = dbs[0];
        let g = if t.contains("darker") {
            -d.abs()
        } else if t.contains("brighter") {
            d.abs()
        } else {
            d
        };
        return Some(step(
            "tilt",
            json!({ "db_per_octave": g }),
            scope,
            format!("Tilt the spectrum by {g:+} dB per octave around 1 kHz, {whole}."),
        ));
    }

    // A gate: "gate the pauses", "gate below -50 dB".
    if has_word(t, "gate") || has_word(t, "gating") {
        let (params, words) = match dbs.first() {
            Some(d) => (
                json!({ "threshold_dbfs": -d.abs() }),
                format!("{} dBFS", -d.abs()),
            ),
            None => (json!({}), "6 dB above the noise floor".to_string()),
        };
        return Some(step(
            "gate",
            params,
            scope,
            format!("Lower the level by up to 12 dB where it falls below {words}, {whole}."),
        ));
    }

    // Hum removal: the mains hum and its harmonics.
    if has_any(
        t,
        &[
            "dehum",
            "de-hum",
            "mains",
            "hum and its harmonics",
            "hum and harmonics",
            "harmonics of the hum",
            "all the harmonics",
        ],
    ) {
        let base = freqs
            .iter()
            .find(|f| (**f - 50.0).abs() < 1.0 || (**f - 60.0).abs() < 1.0)
            .map(|f| if *f < 55.0 { "50" } else { "60" });
        let (params, words) = match base {
            Some(b) => (json!({ "fundamental": b }), format!("{b} Hz")),
            None => (json!({}), "the mains frequency found".to_string()),
        };
        return Some(step(
            "hum_reduce",
            params,
            scope,
            format!("Cut the hum at {words} and its first 8 harmonics by 30 dB, {whole}."),
        ));
    }
    None
}

fn step(op: &str, params: Value, scope: Scope, understood: String) -> Route {
    Route::Steps {
        steps: vec![RoutedStep {
            op: op.into(),
            params,
            scope,
            understood,
        }],
    }
}

fn scope_words(scope: &Scope) -> String {
    match scope {
        Scope::Clip => "across the whole recording".into(),
        Scope::TimeRange { t0, t1 } => format!("from {t0:.2} to {t1:.2} s"),
        Scope::Band { f_lo, f_hi } => format!("between {f_lo:.0} and {f_hi:.0} Hz"),
        Scope::TfPatch { t0, t1, f_lo, f_hi } => {
            format!("in the selected area ({t0:.2}–{t1:.2} s, {f_lo:.0}–{f_hi:.0} Hz)")
        }
    }
}

/// Route a request. `selection` is what "here" or "the selection" refers to.
pub fn route(words: &str, selection: Option<&Selection>) -> Route {
    let t = normalise(words);
    let nums = numbers(&t);
    let freqs = frequencies(&nums);
    let dbs = decibels(&nums);
    let here = has_any(
        &t,
        &[
            " here",
            "this bit",
            "this part",
            "selection",
            "selected",
            "this area",
        ],
    );
    let area = if here {
        selection.and_then(Selection::scope)
    } else {
        None
    };
    let span = if here {
        selection.and_then(Selection::time_scope)
    } else {
        None
    };

    // What to listen to.
    if has_any(
        &t,
        &["play", "listen", "hear", "compare", "switch to", "show me"],
    ) {
        if has_any(&t, &["residual", "removed", "taken out", "what was cut"]) {
            return Route::Listen {
                which: "residual".into(),
            };
        }
        if has_any(&t, &["original", "before", "untouched", "source"]) {
            return Route::Listen {
                which: "source".into(),
            };
        }
        if has_any(&t, &["processed", "result", "cleaned", "after"]) {
            return Route::Listen {
                which: "stack".into(),
            };
        }
    }

    if let Some(r) = time_edit(&t, &nums, selection) {
        return r;
    }

    if t == "undo"
        || t.starts_with("undo ")
        || has_any(
            &t,
            &[
                "remove the last step",
                "take that back",
                "take it back",
                "undo that",
            ],
        )
    {
        return Route::Undo;
    }
    if t == "redo" || t.starts_with("redo ") {
        return Route::Redo;
    }

    if let Some(r) = catalogue(&t, &nums, &freqs, &dbs, span.clone()) {
        return r;
    }

    let cutting = has_any(
        &t,
        &[
            "cut",
            "notch",
            "remove",
            "reduce",
            "attenuate",
            "get rid of",
            "lower",
            "kill",
            "clean",
        ],
    );

    // A stated band: "cut 3,100 to 3,200 Hz by 12 dB".
    if cutting && freqs.len() >= 2 && t.contains(" to ") {
        let (lo, hi) = (freqs[0].min(freqs[1]), freqs[0].max(freqs[1]));
        let mut params = json!({ "f_lo": lo, "f_hi": hi });
        let depth = dbs.first().map(|d| d.abs());
        if let Some(d) = depth {
            params["depth_db"] = json!(d);
        }
        let scope = span.unwrap_or(Scope::Clip);
        let how = depth.map_or("by the default 12 dB".to_string(), |d| format!("by {d} dB"));
        return step(
            "band_cut",
            params,
            scope.clone(),
            format!("Cut {lo:.0}–{hi:.0} Hz {how}, {}.", scope_words(&scope)),
        );
    }

    // One stated frequency: "remove the 750 Hz line".
    if cutting && freqs.len() == 1 {
        let f = freqs[0];
        let half = (0.03 * f).max(25.0);
        let scope = Scope::Band {
            f_lo: (f - half).max(1.0),
            f_hi: f + half,
        };
        return step(
            "line_reduce",
            json!({ "lines": "auto", "require_in_pauses": false }),
            scope.clone(),
            format!(
                "Find the line near {f:.0} Hz and cut it until it matches its surroundings ({}).",
                scope_words(&scope)
            ),
        );
    }

    if has_any(&t, &["normalise", "normalize"]) {
        let target = dbs.first().map(|d| -d.abs()).unwrap_or(-1.0);
        let scope = span.unwrap_or(Scope::Clip);
        return step(
            "normalise",
            json!({ "target_peak_dbfs": target }),
            scope.clone(),
            format!("Bring the peak to {target} dBFS, {}.", scope_words(&scope)),
        );
    }

    let louder = has_any(
        &t,
        &[
            "gain",
            "amplify",
            "boost",
            "louder",
            "turn it up",
            "turn up",
            "raise",
        ],
    );
    let quieter = has_any(
        &t,
        &["quieter", "turn it down", "turn down", "lower the level"],
    );
    if (louder || quieter) && !dbs.is_empty() {
        let mut g = dbs[0];
        if quieter && g > 0.0 {
            g = -g;
        }
        let scope = span.unwrap_or(Scope::Clip);
        return step(
            "gain",
            json!({ "gain_db": g }),
            scope.clone(),
            format!("Change the level by {g:+} dB, {}.", scope_words(&scope)),
        );
    }

    if has_any(
        &t,
        &["dc offset", "remove dc", "remove the dc", "dc removal"],
    ) {
        return step(
            "dc_remove",
            json!({}),
            Scope::Clip,
            "Remove the DC offset.".into(),
        );
    }

    if cutting && t.contains("hum") {
        let scope = Scope::Band {
            f_lo: 40.0,
            f_hi: 1000.0,
        };
        return step(
            "line_reduce",
            json!({ "lines": "auto" }),
            scope.clone(),
            "Find the hum and its harmonics (40–1000 Hz) and cut each until it matches its surroundings.".into(),
        );
    }

    if (cutting && has_any(&t, &["noise", "hiss"])) || has_any(&t, &["denoise", "noise reduction"])
    {
        let mut params = json!({});
        if let Some(d) = dbs.first() {
            params["reduction_db"] = json!(d.abs());
        }
        let scope = span.unwrap_or(Scope::Clip);
        return step(
            "noise_reduce",
            params,
            scope.clone(),
            format!(
                "Reduce steady noise, with the quietest steady region as the profile, {}.",
                scope_words(&scope)
            ),
        );
    }

    if cutting && has_any(&t, &["lines", "whines", "tones", "whistles"]) {
        return step(
            "line_reduce",
            json!({ "lines": "auto" }),
            Scope::Clip,
            "Find every line that runs through the recording, including its pauses, and flatten it.".into(),
        );
    }

    // The spectral compressor: high points in the selected area.
    if has_any(&t, &["compress", "tame", "squash", "push down", "flatten"])
        && has_any(
            &t,
            &[
                "peak", "bang", "click", "knock", "thump", "whine", "tone", "voice", "loud",
            ],
        )
    {
        let mode = if has_any(&t, &["bang", "click", "knock", "thump"]) {
            "transient"
        } else if has_any(&t, &["whine", "tone", "whistle"]) {
            "tonal"
        } else if has_any(&t, &["voice", "speaker"]) {
            "level"
        } else {
            "auto"
        };
        let scope = area.unwrap_or(Scope::Clip);
        return step(
            "spectral_compressor",
            json!({ "mode": mode }),
            scope.clone(),
            format!(
                "Push down the {} high points {}.",
                if mode == "auto" { "" } else { mode },
                scope_words(&scope)
            )
            .replace("the  high", "the high"),
        );
    }

    if has_any(&t, &["compress", "even out the level", "even out"]) {
        let scope = span.unwrap_or(Scope::Clip);
        return step(
            "compressor",
            json!({}),
            scope.clone(),
            format!("Compress the dynamics, {}.", scope_words(&scope)),
        );
    }

    if (t.contains("clean") && (t.contains(" up") || t.contains("cleanup")))
        || t.contains("clean-up")
        || (t.contains("tidy") && t.contains(" up"))
    {
        return Route::Recipe {
            name: "spoken-word-cleanup".into(),
        };
    }

    Route::Model
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_step(r: Route) -> RoutedStep {
        match r {
            Route::Steps { mut steps } if steps.len() == 1 => steps.remove(0),
            other => panic!("expected one step, got {other:?}"),
        }
    }

    #[test]
    fn routine_requests_are_answered_locally() {
        for w in [
            "clean this recording up",
            "Clean it up please.",
            "can you do a clean-up",
            "tidy this up",
        ] {
            assert_eq!(
                route(w, None),
                Route::Recipe {
                    name: "spoken-word-cleanup".into()
                },
                "{w}"
            );
        }
        assert_eq!(route("undo", None), Route::Undo);
        assert_eq!(route("remove the last step", None), Route::Undo);
        assert_eq!(route("redo", None), Route::Redo);
        assert_eq!(route("redo that", None), Route::Redo);
        assert_eq!(
            route("play what was removed", None),
            Route::Listen {
                which: "residual".into()
            }
        );
        assert_eq!(
            route("compare with the original", None),
            Route::Listen {
                which: "source".into()
            }
        );
    }

    #[test]
    fn stated_numbers_become_steps() {
        let s = only_step(route("cut 3,100 to 3,200 Hz by 12 dB", None));
        assert_eq!(s.op, "band_cut");
        assert_eq!(
            s.params,
            json!({"f_lo": 3100.0, "f_hi": 3200.0, "depth_db": 12.0})
        );
        assert_eq!(s.scope, Scope::Clip);

        let s = only_step(route("notch 3.1-3.2 kHz", None));
        assert_eq!(
            (s.op.as_str(), &s.params["f_lo"], &s.params["f_hi"]),
            ("band_cut", &json!(3100.0), &json!(3200.0))
        );

        let s = only_step(route("remove the 750 Hz whine", None));
        assert_eq!(s.op, "line_reduce");
        assert_eq!(s.params["require_in_pauses"], false);
        assert_eq!(
            s.scope,
            Scope::Band {
                f_lo: 725.0,
                f_hi: 775.0
            }
        );

        let s = only_step(route("amplify by 30 dB", None));
        assert_eq!(
            (s.op.as_str(), &s.params),
            ("gain", &json!({"gain_db": 30.0}))
        );
        let s = only_step(route("make it quieter by 3 dB", None));
        assert_eq!(s.params, json!({"gain_db": -3.0}));
        let s = only_step(route("normalise to -1 dB", None));
        assert_eq!(s.params, json!({"target_peak_dbfs": -1.0}));
        let s = only_step(route("reduce the noise by 18 dB", None));
        assert_eq!(
            (s.op.as_str(), &s.params),
            ("noise_reduce", &json!({"reduction_db": 18.0}))
        );
        assert_eq!(only_step(route("remove the hum", None)).op, "line_reduce");
        assert_eq!(
            only_step(route("remove the DC offset", None)).op,
            "dc_remove"
        );
        // A specific target wins over the whole clean-up.
        assert_eq!(only_step(route("clean up the hum", None)).op, "line_reduce");
    }

    #[test]
    fn here_means_the_selection() {
        let sel = Selection {
            time: Some([1.0, 2.0]),
            area: Some([1.5, 3.0, 700.0, 900.0]),
        };
        let s = only_step(route("compress the bangs here", Some(&sel)));
        assert_eq!(s.op, "spectral_compressor");
        assert_eq!(s.params["mode"], "transient");
        assert_eq!(
            s.scope,
            Scope::TfPatch {
                t0: 1.5,
                t1: 3.0,
                f_lo: 700.0,
                f_hi: 900.0
            }
        );
        let s = only_step(route("boost the selection by 6 dB", Some(&sel)));
        assert_eq!(s.scope, Scope::TimeRange { t0: 1.0, t1: 2.0 });
        // Without "here", the selection is not used.
        let s = only_step(route("compress the peaks", Some(&sel)));
        assert_eq!(s.scope, Scope::Clip);
    }

    #[test]
    fn time_edits_by_their_numbers_or_the_selection() {
        let r = |w: &str| only_step(route(w, None));
        let s = r("remove 12 to 15.5 seconds");
        assert_eq!(
            (s.op.as_str(), s.scope.clone()),
            ("remove_time", Scope::TimeRange { t0: 12.0, t1: 15.5 })
        );
        assert_eq!(
            r("cut from 1:20 to 1:35").scope,
            Scope::TimeRange { t0: 80.0, t1: 95.0 }
        );
        assert_eq!(
            r("delete between 2 s and 2500 ms").scope,
            Scope::TimeRange { t0: 2.0, t1: 2.5 }
        );
        assert_eq!(
            r("trim the first 5 seconds").scope,
            Scope::TimeRange { t0: 0.0, t1: 5.0 }
        );

        let s = r("insert 2 seconds of silence at 30 s");
        assert_eq!(s.op, "insert_silence");
        assert_eq!(s.params, json!({ "at_s": 30.0, "duration_s": 2.0 }));
        assert_eq!(
            r("add a 500 ms gap at 1:05").params,
            json!({ "at_s": 65.0, "duration_s": 0.5 })
        );
        assert_eq!(r("pad 2 s of silence at the start").params["at_s"], 0.0);

        let sel = Selection {
            time: Some([2.0, 3.0]),
            area: None,
        };
        let s = only_step(route("remove this part", Some(&sel)));
        assert_eq!(
            (s.op.as_str(), s.scope),
            ("remove_time", Scope::TimeRange { t0: 2.0, t1: 3.0 })
        );
        assert_eq!(
            only_step(route("cut the selection out", Some(&sel))).op,
            "remove_time"
        );
        let s = only_step(route("add 1 s of silence here", Some(&sel)));
        assert_eq!(s.params, json!({ "at_s": 2.0, "duration_s": 1.0 }));

        // Naming something to process is processing, not a cut in time.
        assert_eq!(
            only_step(route("remove the hum here", Some(&sel))).op,
            "line_reduce"
        );
        assert_eq!(r("cut 3,100 to 3,200 Hz by 12 dB").op, "band_cut");
        assert_eq!(r("remove the 750 Hz line").op, "line_reduce");
        // Not enough to go on: the model decides.
        for w in [
            "insert some silence",
            "remove the last 3 seconds",
            "cut this part",
        ] {
            assert_eq!(route(w, None), Route::Model, "{w}");
        }
    }

    #[test]
    fn spoken_forms_route_as_typed_ones_do() {
        // As a recogniser writes them: capitals, full stops, words for signs and units.
        let r = |w: &str| only_step(route(w, None));
        assert_eq!(
            r("Normalise to minus 1 dB.").params,
            json!({"target_peak_dbfs": -1.0})
        );
        assert_eq!(
            r("Normalize to negative 3 decibels").params,
            json!({"target_peak_dbfs": -3.0})
        );
        let s = r("Cut 3.1 to 3.2 kilohertz by 12 dB.");
        assert_eq!(
            (s.op.as_str(), &s.params),
            (
                "band_cut",
                &json!({"f_lo": 3100.0, "f_hi": 3200.0, "depth_db": 12.0})
            )
        );
        let s = r("Cut 3,100 to 3,200 hertz by 12 decibels.");
        assert_eq!(
            s.params,
            json!({"f_lo": 3100.0, "f_hi": 3200.0, "depth_db": 12.0})
        );
        assert_eq!(
            r("Remove 12 to 15.5 seconds.").scope,
            Scope::TimeRange { t0: 12.0, t1: 15.5 }
        );
        assert_eq!(
            r("Insert 1 point 5 seconds of silence at 30 seconds.").params,
            json!({ "at_s": 30.0, "duration_s": 1.5 })
        );
        assert_eq!(r("Remove the 750 hertz line.").op, "line_reduce");
        assert_eq!(r("Turn it up by 3 dB.").params, json!({"gain_db": 3.0}));
        assert_eq!(route("Undo.", None), Route::Undo);
        assert_eq!(route("Redo.", None), Route::Redo);
        assert_eq!(
            route("Clean this recording up.", None),
            Route::Recipe {
                name: "spoken-word-cleanup".into()
            }
        );
        assert_eq!(
            route("Play the residual.", None),
            Route::Listen {
                which: "residual".into()
            }
        );
        // "At this point" is not a number.
        assert_eq!(
            normalise("add a pause at this point, please."),
            "add a pause at this point, please"
        );
    }

    #[test]
    fn the_catalogue_by_its_names_and_numbers() {
        let r = |w: &str| only_step(route(w, None));
        let is = |w: &str, op: &str, params: Value| {
            let s = r(w);
            assert_eq!((s.op.as_str(), &s.params), (op, &params), "{w}");
        };
        is(
            "high-pass at 80 Hz",
            "high_pass",
            json!({"cutoff_hz": 80.0}),
        );
        is("cut below 100 Hz", "high_pass", json!({"cutoff_hz": 100.0}));
        is(
            "Remove everything below 60 hertz.",
            "high_pass",
            json!({"cutoff_hz": 60.0}),
        );
        is("remove the rumble", "high_pass", json!({}));
        is("low cut", "high_pass", json!({}));
        is(
            "low-pass at 8 kHz",
            "low_pass",
            json!({"cutoff_hz": 8000.0}),
        );
        is(
            "roll off above 12 kHz",
            "low_pass",
            json!({"cutoff_hz": 12000.0}),
        );
        is(
            "high cut at 10 kHz",
            "low_pass",
            json!({"cutoff_hz": 10000.0}),
        );
        is(
            "boost 3 kHz by 3 dB",
            "bell",
            json!({"freq_hz": 3000.0, "gain_db": 3.0}),
        );
        is(
            "dip 250 Hz by 4 dB",
            "bell",
            json!({"freq_hz": 250.0, "gain_db": -4.0}),
        );
        is(
            "boost the bass by 3 dB",
            "shelf",
            json!({"kind": "low", "freq_hz": 200.0, "gain_db": 3.0}),
        );
        is(
            "cut the treble above 5 kHz by 4 dB",
            "shelf",
            json!({"kind": "high", "freq_hz": 5000.0, "gain_db": -4.0}),
        );
        is(
            "make it brighter by 1.5 dB per octave",
            "tilt",
            json!({"db_per_octave": 1.5}),
        );
        is(
            "darker by 1 dB per octave",
            "tilt",
            json!({"db_per_octave": -1.0}),
        );
        is(
            "normalise to -16 LUFS",
            "loudness_normalise",
            json!({"target_lufs": -16.0}),
        );
        is(
            "Normalize the loudness to minus 19.",
            "loudness_normalise",
            json!({"target_lufs": -19.0}),
        );
        is("normalise the loudness", "loudness_normalise", json!({}));
        is("gate the pauses", "gate", json!({}));
        is(
            "gate below -50 dB",
            "gate",
            json!({"threshold_dbfs": -50.0}),
        );
        is("dehum", "hum_reduce", json!({}));
        is("remove the mains hum", "hum_reduce", json!({}));
        is(
            "remove the 60 Hz hum and its harmonics",
            "hum_reduce",
            json!({"fundamental": "60"}),
        );
        // What was already understood stays as it was.
        is(
            "normalise to -1 dB",
            "normalise",
            json!({"target_peak_dbfs": -1.0}),
        );
        is("boost it by 6 dB", "gain", json!({"gain_db": 6.0}));
        assert_eq!(r("remove the hum").op, "line_reduce");
        assert_eq!(r("remove the 750 Hz line").op, "line_reduce");
        // Short words only as words.
        assert_eq!(
            route("investigate the noise at the start", None),
            Route::Model
        );
        // Here: the selection.
        let sel = Selection {
            time: Some([1.0, 2.0]),
            area: None,
        };
        let s = only_step(route("high-pass here at 100 Hz", Some(&sel)));
        assert_eq!(s.scope, Scope::TimeRange { t0: 1.0, t1: 2.0 });
        // Not enough to go on: the model decides.
        for w in ["low-pass it", "make it brighter", "boost the presence"] {
            assert_eq!(route(w, None), Route::Model, "{w}");
        }
    }

    #[test]
    fn anything_else_goes_to_the_model() {
        for w in [
            "the hum is distracting",
            "make the quiet voice easier to hear",
            "what is that noise at the start?",
            "",
        ] {
            assert_eq!(route(w, None), Route::Model, "{w}");
        }
    }
}
