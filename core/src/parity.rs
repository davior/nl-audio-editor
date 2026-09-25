//! Cross-target parity. A short golden clip is resolved and rendered through
//! every kind of operation, analysis included, then time edits are applied
//! before the final limiter, as an export does; the hash of the result is pinned
//! here. The native test suite and the WebAssembly test suite both assert it,
//! so any arithmetic difference between the two builds fails one of them.

use serde_json::{json, Value};

use crate::audio::AudioBuffer;
use crate::engine::{render_full, RenderStep};
use crate::golden;
use crate::hash::sha256;
use crate::ops::registry;
use crate::provenance::jcs;
use crate::scope::Scope;
use crate::timeline;

/// SHA-256 of the canonical resolved chain, then the render hash of its output.
pub const EXPECTED_RESOLVED: &str =
    "sha256:e2d18b971c09b233b9541d540d8d9c9a8f1f3bef12ca8cfb19a7b1e970f58689";
pub const EXPECTED_RENDER: &str =
    "sha256:bd277ae3869db87492f7d5363029f11eee80bc7500d9ce587c512f850007c356";

pub fn fixture_clip() -> AudioBuffer {
    let mut s = golden::spec_a();
    s.duration_s = 6.0;
    s.long_pause = (3.5, 5.0);
    s.whine_spans = vec![(1.0, 2.0)];
    s.bang_times = vec![2.6];
    s.clip_span = (5.4, 5.6);
    golden::generate(&s).mix
}

/// Resolve and render the parity chain; returns (resolved hash, render hash).
pub fn run() -> (String, String) {
    let input = fixture_clip();
    let chain: Vec<(&str, Value, Scope)> = vec![
        ("dc_remove", json!({}), Scope::Clip),
        ("noise_reduce", json!({}), Scope::Clip),
        ("line_reduce", json!({}), Scope::Clip),
        (
            "spectral_compressor",
            json!({"mode": "auto"}),
            Scope::TfPatch {
                t0: 0.5,
                t1: 3.0,
                f_lo: 3000.0,
                f_hi: 7000.0,
            },
        ),
        (
            "band_cut",
            json!({"f_lo": 3100, "f_hi": 3200, "depth_db": 6}),
            Scope::Clip,
        ),
        ("normalise", json!({}), Scope::Clip),
        ("compressor", json!({}), Scope::Clip),
        (
            "gain",
            json!({"gain_db": 2}),
            Scope::TimeRange { t0: 1.0, t1: 2.0 },
        ),
        (
            "remove_time",
            json!({"fade_ms": 5}),
            Scope::TimeRange { t0: 1.25, t1: 1.9 },
        ),
        (
            "insert_silence",
            json!({"at_s": 4.2, "duration_s": 0.3}),
            Scope::Clip,
        ),
        ("limiter", json!({}), Scope::Clip),
    ];
    let reg = registry();
    let mut steps: Vec<RenderStep> = Vec::new();
    let mut cur = input;
    for (op, params, scope) in chain {
        // The latest version of every operation, so new code paths are covered.
        let version = reg.latest(op).expect("op").descriptor().version;
        let p = reg
            .validate(op, version, &params, &scope)
            .expect("valid parity params");
        let resolved = reg
            .get(op, version)
            .expect("op")
            .resolve(&p, &scope, &cur)
            .expect("resolves");
        let step = RenderStep {
            op: op.into(),
            op_version: version,
            resolved,
            scope,
        };
        if op == "limiter" {
            // As in an export: the time edits, then the limiter.
            cur = timeline::layout(&timeline::edits(&steps), cur.len()).apply(&cur);
        }
        cur = render_full(&cur, std::slice::from_ref(&step))
            .expect("renders")
            .audio;
        steps.push(step);
    }
    let resolved =
        jcs::canonical_bytes(&serde_json::to_value(&steps).expect("json")).expect("finite");
    (sha256(&resolved), cur.render_hash())
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_render_matches_the_pinned_hashes() {
        let (resolved, render) = super::run();
        println!("resolved {resolved}\nrender   {render}");
        assert_eq!(resolved, super::EXPECTED_RESOLVED);
        assert_eq!(render, super::EXPECTED_RENDER);
    }
}
