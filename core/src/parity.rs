//! Cross-target parity. A short golden clip is resolved and rendered through
//! every kind of operation, analysis included; the hash of the result is pinned
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

/// SHA-256 of the canonical resolved chain, then the render hash of its output.
pub const EXPECTED_RESOLVED: &str =
    "sha256:d42e73a5e15063f9d439317f1ad146687944fcbc8cc830419dc2f663fafaa8b1";
pub const EXPECTED_RENDER: &str =
    "sha256:8060abc87eddc8245c2cf350e9809a74dfc9c283616fef73438610a19a3787b9";

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
