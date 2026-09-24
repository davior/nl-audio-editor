//! Human-readable summaries of steps, previews and reports.

use nlae_core::project::bindings::DiffEntry;
use nlae_core::project::{OpenReport, Step};
use nlae_core::scope::Scope;
use serde_json::Value;

pub fn scope(s: &Scope) -> String {
    match s {
        Scope::Clip => "whole clip".into(),
        Scope::TimeRange { t0, t1 } => format!("{t0:.2}–{t1:.2} s"),
        Scope::Band { f_lo, f_hi } => format!("{f_lo:.0}–{f_hi:.0} Hz"),
        Scope::TfPatch { t0, t1, f_lo, f_hi } => {
            format!("{t0:.2}–{t1:.2} s × {f_lo:.0}–{f_hi:.0} Hz")
        }
    }
}

fn num(v: &Value, digits: usize) -> String {
    v.as_f64()
        .map(|x| format!("{x:.digits$}"))
        .unwrap_or_else(|| v.to_string())
}

/// The values that matter for each operation, in one line.
pub fn resolved(step: &Step) -> String {
    let r = &step.resolved;
    match step.op.as_str() {
        "gain" => format!("{} dB", num(&r["gain_db"], 2)),
        "normalise" => format!(
            "peak {} → {} dBFS ({} dB)",
            num(&r["measured_peak_dbfs"], 2),
            num(&r["target_peak_dbfs"], 1),
            num(&r["gain_db"], 2)
        ),
        "dc_remove" => match r["mode"].as_str() {
            Some("mean") => format!(
                "offset {}",
                r["offsets"]
                    .as_array()
                    .map(|a| a.iter().map(|v| num(v, 6)).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()
            ),
            _ => format!("drift, {} ms window", num(&r["window_ms"], 0)),
        },
        "noise_reduce" => format!(
            "profile {}–{} s{}, reduction {} dB, sensitivity {} dB",
            num(&r["profile_range"]["t0"], 2),
            num(&r["profile_range"]["t1"], 2),
            if r["profile_range"]["auto"] == true {
                " (quietest steady region)"
            } else {
                ""
            },
            num(&r["reduction_db"], 1),
            num(&r["sensitivity_db"], 1)
        ),
        "line_reduce" => {
            let lines = r["resolved_lines"].as_array().cloned().unwrap_or_default();
            let list: Vec<String> = lines
                .iter()
                .map(|l| {
                    format!(
                        "{} Hz −{} dB",
                        num(&l["freq_hz"], 1),
                        num(&l["depth_db"], 1)
                    )
                })
                .collect();
            format!(
                "{} line(s): {}",
                lines.len(),
                if list.is_empty() {
                    "none found".into()
                } else {
                    list.join(", ")
                }
            )
        }
        "band_cut" => format!(
            "{}–{} Hz −{} dB",
            num(&r["f_lo"], 1),
            num(&r["f_hi"], 1),
            num(&r["depth_db"], 1)
        ),
        "spectral_compressor" => format!(
            "{} mode, threshold {} dB, ratio {}, max −{} dB",
            r["mode"].as_str().unwrap_or("?"),
            num(&r["threshold_db"], 1),
            num(&r["ratio"], 1),
            num(&r["max_reduction_db"], 1)
        ),
        "compressor" => format!(
            "threshold {} dBFS, ratio {}, make-up {} dB",
            num(&r["threshold_dbfs"], 1),
            num(&r["ratio"], 1),
            num(&r["makeup_db"], 1)
        ),
        _ => serde_json::to_string(r).unwrap_or_default(),
    }
}

pub fn measurements(step: &Step) -> String {
    step.measurements
        .iter()
        .filter(|(_, v)| v.is_number())
        .map(|(k, v)| format!("{k} {}", num(v, 2)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn step_line(i: usize, s: &Step) -> String {
    let who = match (&s.actor.model, s.origin) {
        (Some(m), o) => format!("{m} via {o:?}").to_lowercase(),
        (None, o) => format!("{:?} via {o:?}", s.actor.kind).to_lowercase(),
    };
    format!(
        "{:>2}. {:<20} {}  [{}; {}]",
        i + 1,
        s.op,
        resolved(s),
        scope(&s.scope),
        who
    )
}

pub fn diff(entries: &[DiffEntry]) {
    if entries.is_empty() {
        println!("  (no values change)");
        return;
    }
    for d in entries {
        let changed = d.recorded != d.new;
        println!(
            "  step {} {:<20} {:<28} {} → {}{}   {}",
            d.step + 1,
            d.op,
            d.param,
            short(&d.recorded),
            short(&d.new),
            if changed { "" } else { " (unchanged)" },
            d.reason
        );
    }
}

fn short(v: &Value) -> String {
    let s = match v {
        Value::Null => "—".to_string(),
        Value::Number(n) => n
            .as_f64()
            .map(|x| format!("{x:.2}"))
            .unwrap_or_else(|| n.to_string()),
        other => other.to_string(),
    };
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(57).collect::<String>())
    } else {
        s
    }
}

pub fn report(r: &OpenReport) {
    println!(
        "source   {} {}",
        if r.source_ok { "ok " } else { "FAIL" },
        r.source_sha256
    );
    match &r.log.first_failure {
        None => println!(
            "log      ok  {} events, head {}",
            r.log.verified, r.log.head
        ),
        Some(f) => println!(
            "log      FAIL at line {} (seq {}): {}",
            f.line, f.expected_seq, f.reason
        ),
    }
    for l in &r.lineage {
        println!(
            "lineage  {} {} ({} events)",
            if l.verification.ok { "ok " } else { "FAIL" },
            l.project,
            l.verification.verified
        );
    }
    println!(
        "stack    {}",
        if r.projection_ok && r.manifest_consistent {
            "ok  rebuilt from the log, matches the manifest"
        } else {
            "FAIL"
        }
    );
    for p in &r.problems {
        println!("problem  {p}");
    }
    println!("{}", if r.ok() { "VERIFIED" } else { "NOT VERIFIED" });
}
