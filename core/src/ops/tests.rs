//! Invariant tests across every registered operation.

use serde_json::{json, Value};

use super::*;
use crate::audio::AudioBuffer;
use crate::engine::{render_full, render_window, RenderStep};
use crate::math::{sin, PI};
use crate::scope::Scope;

const SR: u32 = 48000;

fn rng(seed: u64) -> impl FnMut() -> f64 {
    let mut s = seed;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// 3 s: noise, a steady 1 kHz tone, a 4.2 kHz burst at 1.0–1.5 s, clicks at 2 s.
fn signal(channels: usize) -> AudioBuffer {
    let len = SR as usize * 3;
    let chans = (0..channels)
        .map(|c| {
            let mut r = rng(11 + c as u64);
            (0..len)
                .map(|i| {
                    let t = i as f64 / SR as f64;
                    let mut x = 0.003 * r() + 0.02 * sin(2.0 * PI * 1000.0 * t);
                    if (1.0..1.5).contains(&t) {
                        x += 0.05 * sin(2.0 * PI * 4200.0 * t);
                    }
                    if (2.0..2.004).contains(&t) {
                        x += 0.3 * r();
                    }
                    x as f32
                })
                .collect()
        })
        .collect();
    AudioBuffer::new(SR, chans)
}

fn step(op: &str, params: Value, scope: Scope, input: &AudioBuffer) -> RenderStep {
    let reg = registry();
    let p = reg.validate(op, 1, &params, &scope).unwrap();
    let resolved = reg.get(op, 1).unwrap().resolve(&p, &scope, input).unwrap();
    RenderStep {
        op: op.into(),
        op_version: 1,
        resolved,
        scope,
    }
}

fn full(input: &AudioBuffer, s: &RenderStep) -> AudioBuffer {
    render_full(input, std::slice::from_ref(s)).unwrap().audio
}

/// Parameters that make each operation do something (used by several tests).
fn active_cases() -> Vec<(&'static str, Value, Scope)> {
    vec![
        (
            "gain",
            json!({"gain_db": -6.0}),
            Scope::TimeRange { t0: 0.5, t1: 2.5 },
        ),
        ("dc_remove", json!({}), Scope::Clip),
        (
            "dc_remove",
            json!({"mode": "drift", "window_ms": 200}),
            Scope::Clip,
        ),
        ("normalise", json!({"target_peak_dbfs": -3}), Scope::Clip),
        (
            "compressor",
            json!({"threshold_dbfs": -40, "ratio": 4}),
            Scope::Clip,
        ),
        ("limiter", json!({"ceiling_dbfs": -20}), Scope::Clip),
        ("noise_reduce", json!({}), Scope::Clip),
        ("line_reduce", json!({}), Scope::Clip),
        (
            "band_cut",
            json!({"f_lo": 950, "f_hi": 1050, "depth_db": 20}),
            Scope::TimeRange { t0: 0.5, t1: 2.5 },
        ),
        (
            "spectral_compressor",
            json!({"mode": "tonal"}),
            Scope::TfPatch {
                t0: 0.8,
                t1: 1.8,
                f_lo: 3000.0,
                f_hi: 6000.0,
            },
        ),
        (
            "spectral_compressor",
            json!({"mode": "transient"}),
            Scope::Clip,
        ),
        (
            "spectral_compressor",
            json!({"mode": "level", "threshold_db": 3}),
            Scope::Band {
                f_lo: 500.0,
                f_hi: 5000.0,
            },
        ),
        (
            "spectral_compressor",
            json!({"mode": "auto", "link_channels": false}),
            Scope::Clip,
        ),
    ]
}

#[test]
fn descriptors_validate_against_the_meta_schema() {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/op-descriptor.schema.json")).unwrap();
    let mut schemas = boon::Schemas::new();
    let mut compiler = boon::Compiler::new();
    compiler
        .add_resource("mem://op-descriptor.schema.json", schema)
        .unwrap();
    let idx = compiler
        .compile("mem://op-descriptor.schema.json", &mut schemas)
        .unwrap();
    for text in DESCRIPTORS {
        let v: Value = serde_json::from_str(text).unwrap();
        if let Err(e) = schemas.validate(&v, idx) {
            panic!("{} does not match the meta-schema: {e}", v["id"]);
        }
    }
}

#[test]
fn every_descriptor_has_an_implementation_and_its_defaults_are_accepted() {
    let reg = registry();
    let input = signal(1);
    let mut ids: Vec<String> = Vec::new();
    for desc in reg.descriptors() {
        ids.push(desc.id.clone());
        // Required parameters get plausible values; everything else is defaulted.
        let mut given = serde_json::Map::new();
        for p in desc.params.iter().filter(|p| p.required) {
            let v = match p.id.as_str() {
                "f_lo" => 900.0,
                "at_s" => 0.5,
                "duration_s" => 0.25,
                _ => 1100.0,
            };
            given.insert(p.id.clone(), json!(v));
        }
        let params = desc.normalise_params(&Value::Object(given)).unwrap();
        assert_eq!(
            params.as_object().unwrap().len(),
            desc.params.len(),
            "{}: every parameter written out",
            desc.id
        );
        let op = reg.get(&desc.id, desc.version).unwrap();
        // The whole clip where allowed; a removal needs a stretch.
        let scope = if desc.scopes.contains(&Scope::Clip.kind()) {
            Scope::Clip
        } else {
            Scope::TimeRange { t0: 0.25, t1: 0.5 }
        };
        assert!(desc.scopes.contains(&scope.kind()));
        let resolved = op
            .resolve(&params, &scope, &input)
            .unwrap_or_else(|e| panic!("{}: {e}", desc.id));
        op.render(
            &resolved,
            &scope,
            &input,
            0,
            0,
            input.len() as i64,
            input.len(),
        )
        .unwrap();
    }
    // Eleven operations; line_reduce has two versions (v1 kept for replays).
    assert_eq!(ids.len(), 12, "descriptors (operation × version)");
    ids.dedup();
    assert_eq!(ids.len(), 11, "operations");
}

#[test]
fn out_of_range_and_unknown_parameters_are_rejected_not_clamped() {
    let reg = registry();
    let bad = [
        ("gain", json!({"gain_db": 61})),
        ("gain", json!({"gain_db": "loud"})),
        ("gain", json!({"volume": 3})),
        ("spectral_compressor", json!({"ratio": 0.5})),
        ("spectral_compressor", json!({"max_reduction_db": 61})),
        ("spectral_compressor", json!({"mode": "magic"})),
        ("noise_reduce", json!({"reduction_db": "auto"})),
        ("noise_reduce", json!({"profile": {"t0": 2, "t1": 1}})),
        (
            "line_reduce",
            json!({"lines": [{"freq_hz": 750, "width_hz": 5, "depth_db": 90}]}),
        ),
        ("band_cut", json!({"f_hi": 3200})),
        ("limiter", json!({"ceiling_dbfs": 1})),
    ];
    for (op, p) in bad {
        let e = reg.validate(op, 1, &p, &Scope::Clip).unwrap_err();
        assert!(matches!(e, OpError::Param(_)), "{op} {p}: {e}");
    }
    // Unsupported scope kinds are rejected too.
    let e = reg
        .validate(
            "limiter",
            1,
            &json!({}),
            &Scope::TimeRange { t0: 0.0, t1: 1.0 },
        )
        .unwrap_err();
    assert!(matches!(e, OpError::ScopeKind { .. }));
}

#[test]
fn identity_settings_render_bit_identical_output() {
    let input = signal(2);
    let cases = vec![
        ("gain", json!({"gain_db": 0}), Scope::Clip),
        (
            "gain",
            json!({"gain_db": 0}),
            Scope::TimeRange { t0: 1.0, t1: 2.0 },
        ),
        (
            "compressor",
            json!({"ratio": 1, "makeup_db": 0}),
            Scope::Clip,
        ),
        ("limiter", json!({"ceiling_dbfs": 0}), Scope::Clip),
        ("noise_reduce", json!({"reduction_db": 0}), Scope::Clip),
        ("line_reduce", json!({"max_depth_db": 0}), Scope::Clip),
        (
            "band_cut",
            json!({"f_lo": 900, "f_hi": 1100, "depth_db": 0}),
            Scope::Clip,
        ),
        ("spectral_compressor", json!({"ratio": 1}), Scope::Clip),
        (
            "spectral_compressor",
            json!({"max_reduction_db": 0}),
            Scope::TfPatch {
                t0: 0.5,
                t1: 2.0,
                f_lo: 100.0,
                f_hi: 8000.0,
            },
        ),
    ];
    for (op, p, scope) in cases {
        let s = step(op, p.clone(), scope, &input);
        assert_eq!(full(&input, &s), input, "{op} {p} is not an identity");
    }
    // Descriptor identity entries are all covered above or are input-dependent.
    for desc in registry().descriptors() {
        for id in &desc.identity {
            let s = step(
                &desc.id,
                Value::Object(id.clone()).clone_with_required(&desc.id),
                Scope::Clip,
                &input,
            );
            assert_eq!(full(&input, &s), input, "{} identity {:?}", desc.id, id);
        }
    }
    // dc_remove on an exactly zero-mean signal, normalise already at target.
    let sym = AudioBuffer::mono(
        SR,
        (0..1000)
            .map(|i| if i % 2 == 0 { 0.25 } else { -0.25 })
            .collect(),
    );
    assert_eq!(
        full(&sym, &step("dc_remove", json!({}), Scope::Clip, &sym)),
        sym
    );
    let at_target = AudioBuffer::mono(SR, vec![0.5, -0.5, 0.25]);
    assert_eq!(
        full(
            &at_target,
            &step(
                "normalise",
                json!({"target_peak_dbfs": crate::math::amp_to_db(0.5)}),
                Scope::Clip,
                &at_target
            )
        ),
        at_target
    );
}

trait WithRequired {
    fn clone_with_required(&self, op: &str) -> Value;
}

impl WithRequired for Value {
    fn clone_with_required(&self, op: &str) -> Value {
        let mut v = self.clone();
        if op == "band_cut" {
            v["f_lo"] = json!(900);
            v["f_hi"] = json!(1100);
        }
        v
    }
}

#[test]
fn region_scoped_operations_leave_everything_else_bit_identical() {
    let input = signal(1);
    let len = input.len();
    for (op, p, scope) in active_cases() {
        let Some((t0, t1)) = scope.time() else {
            continue;
        };
        let s = step(op, p.clone(), scope.clone(), &input);
        let out = full(&input, &s);
        let r = s.op().unwrap().radius(&s.resolved, SR) as i64;
        let (s0, s1) = ((t0 * SR as f64) as i64, (t1 * SR as f64) as i64);
        for i in 0..len as i64 {
            if i < s0 - r || i >= s1 + r {
                assert_eq!(
                    out.channels[0][i as usize].to_bits(),
                    input.channels[0][i as usize].to_bits(),
                    "{op}: sample {i} changed outside the scope"
                );
            }
        }
        assert_ne!(out, input, "{op} {p} did nothing");
    }
}

#[test]
fn previews_equal_the_same_span_of_the_full_render() {
    let input = signal(2);
    let len = input.len() as i64;
    let mut r = rng(5);
    for (op, p, scope) in active_cases() {
        let s = step(op, p.clone(), scope, &input);
        let whole = full(&input, &s);
        for _ in 0..4 {
            let a = ((r() * 0.5 + 0.5) * (len as f64 - 20000.0)) as i64;
            let b = a + 1 + ((r() * 0.5 + 0.5) * 30000.0) as i64;
            let b = b.min(len);
            let w = render_window(&input, std::slice::from_ref(&s), a, b).unwrap();
            assert_eq!(
                w,
                whole.extract_padded(a, b),
                "{op} {p}: preview {a}..{b} differs from the full render"
            );
        }
    }
}

#[test]
fn chains_preview_exactly_too() {
    let input = signal(1);
    let len = input.len() as i64;
    let mut steps = Vec::new();
    let mut cur = input.clone();
    for (op, p, scope) in [
        ("dc_remove", json!({}), Scope::Clip),
        ("normalise", json!({}), Scope::Clip),
        ("noise_reduce", json!({}), Scope::Clip),
        ("line_reduce", json!({}), Scope::Clip),
        (
            "spectral_compressor",
            json!({"mode": "transient"}),
            Scope::Clip,
        ),
        ("compressor", json!({}), Scope::Clip),
        ("limiter", json!({}), Scope::Clip),
    ] {
        let s = step(op, p, scope, &cur);
        cur = full(&cur, &s);
        steps.push(s);
    }
    for (a, b) in [(0, 4800), (len / 3, len / 3 + 48000), (len - 5000, len)] {
        let w = render_window(&input, &steps, a, b).unwrap();
        assert_eq!(w, cur.extract_padded(a, b), "chain preview {a}..{b}");
    }
}

#[test]
fn renders_are_deterministic() {
    let input = signal(2);
    for (op, p, scope) in active_cases() {
        let s1 = step(op, p.clone(), scope.clone(), &input);
        let s2 = step(op, p.clone(), scope, &input);
        assert_eq!(s1.resolved, s2.resolved);
        assert_eq!(
            full(&input, &s1).render_hash(),
            full(&input, &s2).render_hash(),
            "{op}"
        );
    }
}

#[test]
fn attenuative_operations_never_add_energy() {
    let input = signal(1);
    let energy = |b: &AudioBuffer| {
        b.channels[0]
            .iter()
            .map(|&x| (x as f64) * (x as f64))
            .sum::<f64>()
    };
    for (op, p, scope) in active_cases() {
        let s = step(op, p.clone(), scope, &input);
        if s.op().unwrap().descriptor().class != OpClass::Attenuative {
            continue;
        }
        let out = full(&input, &s);
        assert!(
            energy(&out) <= energy(&input) * (1.0 + 1e-6),
            "{op} {p} added energy"
        );
    }
}

#[test]
fn limiter_holds_the_ceiling() {
    let input = signal(1);
    let s = step("limiter", json!({"ceiling_dbfs": -20}), Scope::Clip, &input);
    let out = full(&input, &s);
    let ceiling = crate::math::db_to_amp(-20.0) as f32;
    assert!(out.channels[0]
        .iter()
        .all(|&x| x.abs() <= ceiling * 1.000_001));
}

#[test]
fn spectral_compressor_reduces_the_burst_inside_its_patch_only() {
    let input = signal(1);
    let s = step(
        "spectral_compressor",
        json!({"mode": "tonal"}),
        Scope::TfPatch {
            t0: 0.8,
            t1: 1.8,
            f_lo: 3000.0,
            f_hi: 6000.0,
        },
        &input,
    );
    let out = full(&input, &s);
    let band_energy = |b: &AudioBuffer, f: f64| {
        // Correlate with the tone over 1.1–1.4 s.
        let (a, z) = ((1.1 * SR as f64) as usize, (1.4 * SR as f64) as usize);
        let (mut c, mut q) = (0.0, 0.0);
        for i in a..z {
            let t = i as f64 / SR as f64;
            c += b.channels[0][i] as f64 * sin(2.0 * PI * f * t);
            q += b.channels[0][i] as f64 * crate::math::cos(2.0 * PI * f * t);
        }
        c * c + q * q
    };
    let drop_db = crate::math::power_to_db(band_energy(&input, 4200.0) / band_energy(&out, 4200.0));
    assert!(drop_db > 10.0, "burst reduced by only {drop_db} dB");
    let tone_change =
        crate::math::power_to_db(band_energy(&input, 1000.0) / band_energy(&out, 1000.0));
    assert!(
        tone_change.abs() < 0.01,
        "1 kHz tone outside the patch changed by {tone_change} dB"
    );
}

#[test]
fn line_reduce_v1_replays_as_recorded_and_v2_leaves_the_voice_alone() {
    // The 12 s golden clip: v1 (no pause rule) also cuts two voice harmonics;
    // v2 cuts only the lines that stand out in the pauses too.
    let clip = crate::golden::generate(&crate::golden::spec_a_short()).mix;
    let reg = registry();
    let lines = |version: u32| -> Vec<i64> {
        let p = reg
            .validate("line_reduce", version, &json!({}), &Scope::Clip)
            .unwrap();
        let r = reg
            .get("line_reduce", version)
            .unwrap()
            .resolve(&p, &Scope::Clip, &clip)
            .unwrap();
        let mut f: Vec<i64> = r["resolved_lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["freq_hz"].as_f64().unwrap().round() as i64)
            .collect();
        f.sort();
        f
    };
    assert_eq!(lines(1), vec![50, 100, 150, 584, 750, 1861, 3150]);
    assert_eq!(lines(2), vec![50, 100, 150, 750, 3150]);
    assert_eq!(reg.latest("line_reduce").unwrap().descriptor().version, 2);
}
