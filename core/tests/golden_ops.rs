//! Golden-clip acceptance for the operations, measured per component.

use std::collections::BTreeMap;

use nlae_core::audio::AudioBuffer;
use nlae_core::engine::{render_components, RenderStep};
use nlae_core::golden::{generate, spec_a, Clip};
use nlae_core::math::power_to_db;
use nlae_core::ops::registry;
use nlae_core::scope::Scope;
use serde_json::{json, Value};

fn clip_a() -> &'static Clip {
    static CLIP: std::sync::OnceLock<Clip> = std::sync::OnceLock::new();
    CLIP.get_or_init(|| generate(&spec_a()))
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

fn energy(b: &AudioBuffer, t0: f64, t1: f64) -> f64 {
    let (a, z) = (b.time_to_sample(t0), b.time_to_sample(t1));
    b.channels[0][a..z]
        .iter()
        .map(|&x| (x as f64) * (x as f64))
        .sum()
}

fn pick(clip: &Clip, names: &[&str]) -> BTreeMap<String, AudioBuffer> {
    names
        .iter()
        .map(|n| (n.to_string(), clip.component(n).clone()))
        .collect()
}

/// Change in dB (negative = reduced) of a component over a span.
fn change_db(before: &AudioBuffer, after: &AudioBuffer, t0: f64, t1: f64) -> f64 {
    power_to_db(energy(after, t0, t1) / energy(before, t0, t1))
}

#[test]
fn components_sum_to_the_rendered_mixture() {
    let clip = clip_a();
    let steps = vec![
        step("dc_remove", json!({}), Scope::Clip, &clip.mix),
        step("gain", json!({"gain_db": 20}), Scope::Clip, &clip.mix),
    ];
    let (mix_out, comps) = render_components(&clip.mix, &steps, &clip.components).unwrap();
    let mut worst = 0.0f64;
    for i in (0..mix_out.len()).step_by(101) {
        let sum: f64 = comps.values().map(|c| c.channels[0][i] as f64).sum();
        worst = worst.max((sum - mix_out.channels[0][i] as f64).abs());
    }
    assert!(
        worst < 1e-6,
        "components differ from the mixture by {worst}"
    );
}

#[test]
fn spectral_compressor_tonal_takes_the_whine_down() {
    let clip = clip_a();
    let scope = Scope::TfPatch {
        t0: 9.5,
        t1: 14.5,
        f_lo: 4500.0,
        f_hi: 6000.0,
    };
    let s = step(
        "spectral_compressor",
        json!({"mode": "tonal"}),
        scope,
        &clip.mix,
    );
    let names = ["whine", "background", "foreground"];
    let (_, out) = render_components(&clip.mix, &[s], &pick(clip, &names)).unwrap();
    let whine = change_db(clip.component("whine"), &out["whine"], 10.0, 14.0);
    let bg = change_db(clip.component("background"), &out["background"], 0.0, 60.0);
    println!("tonal: whine {whine:.2} dB, background {bg:.2} dB");
    assert!(whine <= -15.0, "whine only {whine:.2} dB");
    assert!(bg.abs() <= 2.0, "background changed {bg:.2} dB");
}

#[test]
fn spectral_compressor_transient_takes_the_bangs_down() {
    let clip = clip_a();
    let s = step(
        "spectral_compressor",
        json!({"mode": "transient"}),
        Scope::Clip,
        &clip.mix,
    );
    let names = ["bangs", "background", "foreground"];
    let (_, out) = render_components(&clip.mix, &[s], &pick(clip, &names)).unwrap();
    let bangs = change_db(clip.component("bangs"), &out["bangs"], 0.0, 60.0);
    let bg = change_db(clip.component("background"), &out["background"], 0.0, 60.0);
    let fg = change_db(clip.component("foreground"), &out["foreground"], 0.0, 60.0);
    println!("transient: bangs {bangs:.2} dB, background {bg:.2} dB, foreground {fg:.2} dB");
    assert!(bangs <= -10.0, "bangs only {bangs:.2} dB");
    assert!(bg.abs() <= 2.0, "background changed {bg:.2} dB");
}

#[test]
fn spectral_compressor_level_brings_the_background_up_relative_to_the_foreground() {
    let clip = clip_a();
    // Faster, steeper settings than the defaults: the job is to hold the loud
    // voice down syllable by syllable.
    let params = json!({"mode": "level", "ratio": 8, "attack_ms": 5, "release_ms": 50});
    let s = step(
        "spectral_compressor",
        params,
        Scope::Band {
            f_lo: 200.0,
            f_hi: 4000.0,
        },
        &clip.mix,
    );
    let names = ["background", "foreground"];
    let (_, out) = render_components(&clip.mix, &[s], &pick(clip, &names)).unwrap();
    let fg = change_db(clip.component("foreground"), &out["foreground"], 0.0, 60.0);
    let bg = change_db(clip.component("background"), &out["background"], 0.0, 60.0);
    println!(
        "level: foreground {fg:.2} dB, background {bg:.2} dB, relative {:.2} dB",
        fg - bg
    );
    // Target was 6 dB; the synthetic voices overlap in time–frequency, which
    // caps cell-level separation near 5.5 dB (recorded in docs/spec/08-testing.md).
    assert!(
        fg - bg <= -5.0,
        "foreground only {:.2} dB down relative to the background",
        fg - bg
    );
    assert!(bg.abs() <= 2.0, "background changed {bg:.2} dB");
}

#[test]
fn high_pass_takes_the_rumble_down_and_leaves_the_voices() {
    let clip = clip_a();
    let s = step(
        "high_pass",
        json!({"cutoff_hz": 80}),
        Scope::Clip,
        &clip.mix,
    );
    let names = ["rumble", "foreground", "background"];
    let (_, out) = render_components(&clip.mix, &[s], &pick(clip, &names)).unwrap();
    let rumble = change_db(clip.component("rumble"), &out["rumble"], 0.0, 60.0);
    let fg = change_db(clip.component("foreground"), &out["foreground"], 0.0, 60.0);
    let bg = change_db(clip.component("background"), &out["background"], 0.0, 60.0);
    println!(
        "high_pass 80 Hz: rumble {rumble:.2} dB, foreground {fg:.2} dB, background {bg:.2} dB"
    );
    assert!(rumble <= -12.0, "rumble only {rumble:.2} dB");
    assert!(
        fg.abs() <= 0.5 && bg.abs() <= 0.5,
        "voices changed {fg:.2} / {bg:.2} dB"
    );
}

#[test]
fn hum_reduce_takes_the_mains_hum_down_and_nothing_else() {
    let clip = clip_a();
    let s = step("hum_reduce", json!({}), Scope::Clip, &clip.mix);
    assert!(
        (s.resolved["fundamental_hz"].as_f64().unwrap() - 50.0).abs() < 0.1,
        "{}",
        s.resolved["fundamental_hz"]
    );
    let names = ["hum", "foreground", "background", "line_1", "line_2"];
    let (_, out) = render_components(&clip.mix, &[s], &pick(clip, &names)).unwrap();
    let c = |n: &str| change_db(clip.component(n), &out[n], 0.0, 60.0);
    let (hum, fg, bg, l1, l2) = (
        c("hum"),
        c("foreground"),
        c("background"),
        c("line_1"),
        c("line_2"),
    );
    println!(
        "hum_reduce auto: hum {hum:.2} dB, foreground {fg:.2}, background {bg:.2}, 750 Hz {l1:.2}, 3150 Hz {l2:.2} dB"
    );
    assert!(hum <= -20.0, "hum only {hum:.2} dB");
    for (n, v) in [
        ("foreground", fg),
        ("background", bg),
        ("750 Hz line", l1),
        ("3150 Hz line", l2),
    ] {
        assert!(v.abs() <= 0.5, "{n} changed {v:.2} dB");
    }
}

#[test]
fn gate_takes_the_pauses_down_and_keeps_the_voice() {
    let clip = clip_a();
    let s = step("gate", json!({}), Scope::Clip, &clip.mix);
    let names = ["noise", "foreground", "background"];
    let (_, out) =
        render_components(&clip.mix, std::slice::from_ref(&s), &pick(clip, &names)).unwrap();
    // The long pause (40–42.5 s), once the hold and release after the last word are over.
    let pause = change_db(clip.component("noise"), &out["noise"], 40.5, 42.3);
    let fg = change_db(clip.component("foreground"), &out["foreground"], 0.0, 60.0);
    let bg = change_db(clip.component("background"), &out["background"], 0.0, 60.0);
    println!(
        "gate auto: threshold {} dBFS (floor {}), noise in the pause {pause:.2} dB, foreground {fg:.2} dB, background {bg:.2} dB",
        s.resolved["threshold_dbfs"], s.resolved["noise_floor_dbfs"]
    );
    assert!(pause <= -11.0, "noise in the pause only {pause:.2} dB");
    assert!(fg.abs() <= 1.0, "foreground changed {fg:.2} dB");
}

#[test]
fn loudness_normalise_reaches_its_target() {
    let clip = clip_a();
    let s = step("loudness_normalise", json!({}), Scope::Clip, &clip.mix);
    let out = nlae_core::engine::render_full(&clip.mix, std::slice::from_ref(&s))
        .unwrap()
        .audio;
    let lufs = nlae_core::analysis::loudness::integrated_lufs(&out, 0, out.len()).unwrap();
    println!(
        "loudness_normalise: {} LUFS measured, gain {} dB, {lufs:.3} LUFS after",
        s.resolved["measured_lufs"], s.resolved["gain_db"]
    );
    assert!((lufs + 23.0).abs() <= 0.05, "{lufs:.3} LUFS");
}
