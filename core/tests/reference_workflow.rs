//! Acceptance for the primary requirement and the reference workflow
//! (docs/spec/08-testing.md): the automatic spoken-word clean-up on clip A,
//! and the manual Audacity process made reusable on clip B.

use std::collections::BTreeMap;

use nlae_core::analysis::tonal::{LineConfig, LineSpectrum};
use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::audio::AudioBuffer;
use nlae_core::dataset::{self, AudioInclusion, ExportOptions, ProjectData};
use nlae_core::engine::render_components;
use nlae_core::golden::{generate, spec_a, spec_b, Clip};
use nlae_core::math::{amp_to_db, power_to_db};
use nlae_core::project::store::MemStore;
use nlae_core::project::*;
use nlae_core::provenance::{Actor, AppInfo, FixedEnv};
use nlae_core::recipe::{plan_replay, Recipe, ReplayMode};
use nlae_core::scope::Scope;
use serde_json::{json, Value};

fn app() -> AppInfo {
    AppInfo::new("test")
}

fn project(clip: &Clip, name: &str, env: &mut FixedEnv) -> Project<MemStore> {
    let bytes = write_wav(&clip.mix, WavFormat::F32);
    let opts = CreateOptions {
        name: Some(name.into()),
        ..Default::default()
    };
    let mut p = Project::create(
        MemStore::new(),
        env,
        app(),
        &bytes,
        &format!("{name}.wav"),
        opts,
    )
    .unwrap();
    p.analyse(env).unwrap();
    p
}

fn draft(op: &str, params: Value, scope: Scope) -> StepDraft {
    StepDraft::new(op, params, scope, Actor::user(), Origin::Cli)
}

fn apply(p: &mut Project<MemStore>, env: &mut FixedEnv, d: StepDraft) -> Step {
    let pv = p.preview(env, vec![d], PreviewOptions::default()).unwrap();
    p.accept(env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap()
        .remove(0)
}

fn energy(b: &AudioBuffer, t0: f64, t1: f64) -> f64 {
    let (a, z) = (b.time_to_sample(t0), b.time_to_sample(t1));
    b.channels[0][a..z]
        .iter()
        .map(|&x| (x as f64) * (x as f64))
        .sum()
}

/// How far a line stands above its neighbourhood in `audio` (long-term spectrum).
fn prominence_at(audio: &AudioBuffer, freq: f64) -> f64 {
    let spec = LineSpectrum::compute(audio, 0, audio.len());
    let db = spec.db();
    let w = ((LineConfig::default().neighbourhood_hz / spec.bin_hz()).round() as usize).max(4);
    let hood = LineSpectrum::neighbourhood(&db, w);
    spec.prominence_at(&db, &hood, freq)
}

/// Attenuation (dB, positive = reduced) of one golden component across step
/// `k` of a project's stack, with the step's decisions taken from the mixture.
fn attenuation_across(p: &Project<MemStore>, clip: &Clip, k: usize, component: &str) -> f64 {
    let steps: Vec<_> = p.state().steps.iter().map(Step::render_step).collect();
    let comps = BTreeMap::from([(component.to_string(), clip.component(component).clone())]);
    let (_, before) = render_components(&clip.mix, &steps[..k], &comps).unwrap();
    let (_, after) = render_components(&clip.mix, &steps[..=k], &comps).unwrap();
    -power_to_db(energy(&after[component], 0.0, 60.0) / energy(&before[component], 0.0, 60.0))
}

/// Prominence of the detected line nearest `freq` in a step's input features.
fn prominence_going_in(p: &Project<MemStore>, k: usize, freq: f64) -> f64 {
    let f = &p.state().steps[k].state_before.as_ref().unwrap().clip;
    f.tonal_lines
        .iter()
        .filter(|l| (l.freq_hz - freq).abs() < 5.0)
        .map(|l| l.prominence_db)
        .next()
        .unwrap_or(0.0)
}

fn schemas() -> (boon::Schemas, BTreeMap<&'static str, boon::SchemaIndex>) {
    let base = "https://nlae.local/schemas/";
    let files: [(&str, &str); 7] = [
        (
            "step.schema.json",
            include_str!("../../schemas/step.schema.json"),
        ),
        (
            "event.schema.json",
            include_str!("../../schemas/event.schema.json"),
        ),
        (
            "recipe.schema.json",
            include_str!("../../schemas/recipe.schema.json"),
        ),
        (
            "dataset/step-record.schema.json",
            include_str!("../../schemas/dataset/step-record.schema.json"),
        ),
        (
            "dataset/episode-record.schema.json",
            include_str!("../../schemas/dataset/episode-record.schema.json"),
        ),
        (
            "dataset/chat-record.schema.json",
            include_str!("../../schemas/dataset/chat-record.schema.json"),
        ),
        (
            "dataset/manifest.schema.json",
            include_str!("../../schemas/dataset/manifest.schema.json"),
        ),
    ];
    let mut compiler = boon::Compiler::new();
    for (name, text) in files {
        compiler
            .add_resource(
                &format!("{base}{name}"),
                serde_json::from_str(text).unwrap(),
            )
            .unwrap();
    }
    let mut schemas = boon::Schemas::new();
    let mut idx = BTreeMap::new();
    for (name, _) in files {
        idx.insert(
            name,
            compiler
                .compile(&format!("{base}{name}"), &mut schemas)
                .unwrap(),
        );
    }
    (schemas, idx)
}

fn validate(schemas: &boon::Schemas, idx: boon::SchemaIndex, v: &Value, what: &str) {
    if let Err(e) = schemas.validate(v, idx) {
        panic!(
            "{what} does not match its schema: {e}\n{}",
            serde_json::to_string(v)
                .unwrap()
                .chars()
                .take(600)
                .collect::<String>()
        );
    }
}

#[test]
fn automatic_reference_workflow_on_clip_a() {
    let clip = generate(&spec_a());
    let mut env = FixedEnv::default();
    let mut p = project(&clip, "golden_a", &mut env);
    let recipe = Recipe::builtin("spoken-word-cleanup").unwrap();
    let plan = plan_replay(&mut p, &recipe, ReplayMode::Adaptive, false, Actor::user()).unwrap();
    assert_eq!(plan.drafts.len(), 4, "compressor is optional and off");
    for d in &plan.diff {
        println!(
            "diff: step {} {} {}: {} -> {} ({})",
            d.step, d.op, d.param, d.recorded, d.new, d.reason
        );
    }
    let recipe_ref = json!({ "name": plan.recipe, "hash": plan.recipe_hash, "mode": "adaptive" });
    let pv = p
        .preview(
            &mut env,
            plan.drafts.clone(),
            PreviewOptions {
                recipe: Some(recipe_ref),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(pv.record.kind, "plan", "previewed as one unit");
    let steps = p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    assert_eq!(steps.len(), 4);

    // Stack output (before the final limiter).
    let out = p.current_render().unwrap();
    let mean: f64 = out.channels[0].iter().map(|&x| x as f64).sum::<f64>() / out.len() as f64;
    let peak = out.channels[0].iter().fold(0.0f32, |m, &x| m.max(x.abs())) as f64;
    println!("dc {mean:.2e}, peak {:.3} dBFS", amp_to_db(peak));
    assert!(mean.abs() < 1e-4, "DC {mean}");
    assert!(
        (amp_to_db(peak) + 1.0).abs() <= 0.1,
        "peak {:.3} dBFS",
        amp_to_db(peak)
    );
    let fin = p.render_final().unwrap();
    println!("limiter: {:?}", fin.limiter);
    assert_eq!(fin.limiter["samples_limited"], 0, "the limiter did nothing");

    // Per component: noise down in the pause, voices kept.
    let gain_db = steps[3].resolved["gain_db"].as_f64().unwrap();
    let rsteps: Vec<_> = steps.iter().map(Step::render_step).collect();
    let names = ["noise", "rumble", "foreground", "background"];
    let comps: BTreeMap<String, AudioBuffer> = names
        .iter()
        .map(|n| (n.to_string(), clip.component(n).clone()))
        .collect();
    let (_, outc) = render_components(&clip.mix, &rsteps, &comps).unwrap();
    let change = |n: &str, t0, t1| {
        power_to_db(energy(&outc[n], t0, t1) / energy(clip.component(n), t0, t1)) - gain_db
    };
    let noise = power_to_db(
        (energy(&outc["noise"], 40.2, 42.3) + energy(&outc["rumble"], 40.2, 42.3))
            / (energy(clip.component("noise"), 40.2, 42.3)
                + energy(clip.component("rumble"), 40.2, 42.3)),
    ) - gain_db;
    let (fg, bg) = (
        change("foreground", 0.0, 60.0),
        change("background", 0.0, 60.0),
    );
    println!("noise in the pause {noise:.2} dB, foreground {fg:.2} dB, background {bg:.2} dB");
    assert!(noise <= -10.0, "noise only {noise:.2} dB down");
    assert!(
        fg.abs() <= 2.0 && bg.abs() <= 2.0,
        "voices changed: fg {fg:.2}, bg {bg:.2}"
    );

    // No line stands out any more (the manual stop rule, measured).
    let remaining = steps[2].measurements["max_remaining_prominence_db"]
        .as_f64()
        .unwrap();
    println!("lines found: {}", steps[2].resolved["resolved_lines"]);
    println!("largest remaining prominence {remaining:.2} dB");
    assert!(remaining <= 3.0);
    for f in [50.0, 100.0, 750.0, 3150.0] {
        let pr = prominence_at(&out, f);
        println!("line {f} Hz: {pr:.2} dB");
        assert!(pr <= 3.0, "line at {f} Hz still stands out by {pr:.2} dB");
    }
}

#[test]
fn manual_process_becomes_reusable_on_another_clip() {
    let (schemas, idx) = schemas();
    let clip_a = generate(&spec_a());
    let clip_b = generate(&spec_b());
    let mut env = FixedEnv::default();

    // --- The Audacity process, by hand, on clip A (with a rejection and a change of mind).
    let mut a = project(&clip_a, "golden_a", &mut env);
    let pv = a
        .preview(
            &mut env,
            vec![draft("gain", json!({"gain_db": 40}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    a.reject(
        &mut env,
        &pv.record.preview_id,
        Some("too loud".into()),
        None,
    )
    .unwrap();
    apply(
        &mut a,
        &mut env,
        draft("gain", json!({"gain_db": 30}), Scope::Clip),
    );
    apply(&mut a, &mut env, draft("dc_remove", json!({}), Scope::Clip));
    let pv = a
        .preview(
            &mut env,
            vec![draft(
                "normalise",
                json!({"target_peak_dbfs": -3}),
                Scope::Clip,
            )],
            PreviewOptions::default(),
        )
        .unwrap();
    let mut opts = AcceptOptions::default();
    opts.overrides.insert(0, json!({"target_peak_dbfs": -1}));
    a.accept(&mut env, &pv.record.preview_id, opts).unwrap();
    apply(
        &mut a,
        &mut env,
        draft(
            "noise_reduce",
            json!({"profile": {"t0": 40.4, "t1": 41.4}}),
            Scope::Clip,
        ),
    );
    // Read the lines off the current analysis, as a person reads the spectrogram.
    let cur = a.current_render().unwrap();
    let feats = a.features_for(&cur);
    let prom = |f: f64| {
        feats
            .tonal_lines
            .iter()
            .find(|l| (l.freq_hz - f).abs() < 3.0)
            .map(|l| l.prominence_db)
            .unwrap()
    };
    let (p750, p3150) = (prom(750.0).round(), prom(3150.0).round());
    let mut cut1 = draft(
        "band_cut",
        json!({"f_lo": 740, "f_hi": 760, "depth_db": p750}),
        Scope::Clip,
    );
    cut1.intent = Some("notch the line at about 750 Hz until it matches the surroundings".into());
    let s_cut1 = apply(&mut a, &mut env, cut1);
    let mut cut2 = draft(
        "band_cut",
        json!({"f_lo": 3100, "f_hi": 3200, "depth_db": p3150}),
        Scope::Clip,
    );
    cut2.intent = Some("inside 3–4 kHz, reduce only 3,100–3,200 Hz".into());
    let s_cut2 = apply(&mut a, &mut env, cut2);

    // Inferred bindings point at the detected lines.
    for (s, f) in [(&s_cut1, 750.0), (&s_cut2, 3150.0)] {
        let near = s.bindings["f_lo"].select.as_ref().unwrap()["near_hz"]
            .as_f64()
            .unwrap();
        assert!((near - f).abs() < 2.0, "{} bound to {near}", s.op);
    }
    let nr = &a.state().steps[3];
    assert_eq!(nr.bindings["profile"].feature, "quietest_region");
    assert_eq!(a.state().steps[0].bindings["gain_db"].relation, "target");

    let recipe = Recipe::from_project(&a, "manual-cleanup", 0, 5).unwrap();
    let recipe_json = serde_json::to_value(&recipe).unwrap();
    validate(
        &schemas,
        idx["recipe.schema.json"],
        &recipe_json,
        "saved recipe",
    );
    validate(
        &schemas,
        idx["recipe.schema.json"],
        &serde_json::from_str(nlae_core::recipe::BUILTIN_SPOKEN_WORD_CLEANUP).unwrap(),
        "built-in recipe",
    );
    a.record(
        &mut env,
        "recipe.saved",
        None,
        json!({ "name": recipe.name, "hash": recipe.hash(), "steps": 6 }),
    )
    .unwrap();

    // --- Adaptive replay on clip B: the cuts move to B's lines.
    let mut b = project(&clip_b, "golden_b", &mut env);
    let plan = plan_replay(&mut b, &recipe, ReplayMode::Adaptive, false, Actor::user()).unwrap();
    for d in &plan.diff {
        println!(
            "diff: step {} {} {}: {} -> {} ({})",
            d.step, d.op, d.param, d.recorded, d.new, d.reason
        );
    }
    let moved = |step: usize, param: &str| {
        plan.diff
            .iter()
            .find(|d| d.step == step && d.param == param)
            .unwrap()
            .new
            .as_f64()
            .unwrap()
    };
    assert!(
        (moved(4, "f_lo") - 610.0).abs() < 3.0 && (moved(4, "f_hi") - 630.0).abs() < 3.0,
        "first cut follows 620 Hz"
    );
    assert!(
        (moved(5, "f_lo") - 2400.0).abs() < 3.0 && (moved(5, "f_hi") - 2500.0).abs() < 3.0,
        "second cut follows 2,450 Hz"
    );
    b.record(
        &mut env,
        "recipe.replayed",
        None,
        json!({ "hash": plan.recipe_hash, "mode": "adaptive", "dry_run": true, "diff": plan.diff }),
    )
    .unwrap();
    let pv = b
        .preview(
            &mut env,
            plan.drafts.clone(),
            PreviewOptions {
                recipe: Some(json!({"hash": plan.recipe_hash, "mode": "adaptive"})),
                ..Default::default()
            },
        )
        .unwrap();
    b.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    // Each of B's lines is cut to (within 3 dB of) its surroundings by the cut that followed it.
    for (k, comp, f) in [(4, "line_1", 620.0), (5, "line_2", 2450.0)] {
        let (att, prom) = (
            attenuation_across(&b, &clip_b, k, comp),
            prominence_going_in(&b, k, f),
        );
        println!("adaptive on B: {f} Hz line stood {prom:.2} dB above its surroundings, cut by {att:.2} dB");
        assert!(prom > 5.0, "the line at {f} Hz was detected going in");
        assert!(
            att >= prom - 3.0,
            "{f} Hz still {:.2} dB above its surroundings",
            prom - att
        );
    }
    let stack = b.state().stack_hash.clone();
    b.rate(
        &mut env,
        Rating {
            target_kind: "stack".into(),
            target: stack,
            overall: 4,
            dims: BTreeMap::from([("intelligibility".to_string(), 4u8)]),
            note: Some("lines gone".into()),
        },
        None,
    )
    .unwrap();

    // --- Exact replay on clip B: the recorded 750 / 3,150 Hz cuts miss B's lines.
    let mut bx = project(&clip_b, "golden_b_exact", &mut env);
    let plan = plan_replay(&mut bx, &recipe, ReplayMode::Exact, false, Actor::user()).unwrap();
    let pv = bx
        .preview(&mut env, plan.drafts, PreviewOptions::default())
        .unwrap();
    bx.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    let out_bx = bx.current_render().unwrap();
    for (k, comp, f) in [(4, "line_1", 620.0), (5, "line_2", 2450.0)] {
        let att = attenuation_across(&bx, &clip_b, k, comp);
        let left = prominence_at(&out_bx, f);
        println!("exact on B: {f} Hz line cut by {att:.2} dB, still stands {left:.2} dB above its surroundings");
        assert!(
            att < 1.0 && left >= 6.0,
            "exact replay should miss B's line at {f} Hz"
        );
    }

    // --- Two clones of B forked after noise reduction, continued differently.
    let fork = b.state().steps[3].step_id.clone();
    let mut c1 = b
        .clone_into(
            &mut env,
            MemStore::new(),
            Some(&fork),
            Some("B stream 1"),
            None,
        )
        .unwrap();
    apply(
        &mut c1,
        &mut env,
        draft(
            "spectral_compressor",
            json!({"mode": "transient"}),
            Scope::Clip,
        ),
    );
    let mut c2 = b
        .clone_into(
            &mut env,
            MemStore::new(),
            Some(&fork),
            Some("B stream 2"),
            None,
        )
        .unwrap();
    apply(
        &mut c2,
        &mut env,
        draft("line_reduce", json!({}), Scope::Clip),
    );

    // --- Every event of every project matches the event schema.
    for proj in [&a, &b, &bx, &c1, &c2] {
        for ev in proj.log.events() {
            validate(
                &schemas,
                idx["event.schema.json"],
                ev,
                &format!("{} event {}", proj.id(), ev["type"]),
            );
        }
    }

    // --- Dataset export.
    let data: Vec<ProjectData> = [&mut a, &mut b, &mut bx, &mut c1, &mut c2]
        .into_iter()
        .map(|p| {
            let src = p.source.clone();
            ProjectData {
                manifest: p.manifest.clone(),
                events: p.log.events().to_vec(),
                source_bytes: Some(p.source_bytes().unwrap()),
                source_features: Some(p.features_for(&src)),
            }
        })
        .collect();
    let ex = dataset::export(
        &data,
        &ExportOptions::default(),
        "2026-09-24T00:00:00.000Z",
        &app(),
    );
    assert!(
        !ex.files.keys().any(|k| k.starts_with("audio/")),
        "no audio unless asked"
    );
    for (name, bytes) in &ex.files {
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            !text.contains("api_key") && !text.to_lowercase().contains("bearer"),
            "{name} carries no keys"
        );
    }
    let lines = |f: &str| -> Vec<Value> {
        String::from_utf8(ex.files[f].clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    let steps = lines("steps.jsonl");
    for r in &steps {
        validate(
            &schemas,
            idx["dataset/step-record.schema.json"],
            r,
            "step record",
        );
    }
    for r in lines("episodes.jsonl") {
        validate(
            &schemas,
            idx["dataset/episode-record.schema.json"],
            &r,
            "episode record",
        );
    }
    let chat = lines("chat.jsonl");
    for r in &chat {
        validate(
            &schemas,
            idx["dataset/chat-record.schema.json"],
            r,
            "chat record",
        );
    }
    validate(
        &schemas,
        idx["dataset/manifest.schema.json"],
        &serde_json::from_slice(&ex.files["manifest.json"]).unwrap(),
        "dataset manifest",
    );
    let outcome = |o: &str| {
        steps
            .iter()
            .filter(|r| r["decision"]["outcome"] == o)
            .count()
    };
    assert!(outcome("rejected") >= 1, "rejections are flagged");
    assert!(steps
        .iter()
        .any(|r| r["decision"]["outcome"] == "rejected" && r["decision"]["applied"] == false));
    assert!(
        steps
            .iter()
            .any(|r| r["decision"]["modification"].is_object()),
        "modifications are flagged"
    );
    assert!(chat.len() >= 2, "steps with words become chat examples");
    let episodes = lines("episodes.jsonl");
    let c1_ep = episodes
        .iter()
        .find(|e| e["project"]["id"] == c1.id())
        .unwrap();
    let rel: Vec<&str> = c1_ep["comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["relation"].as_str().unwrap())
        .collect();
    assert!(
        rel.contains(&"sibling") && rel.contains(&"parent"),
        "clone comparisons linked: {rel:?}"
    );
    println!("dataset counts: {:?}", ex.counts);

    // With audio: the sources, named by hash.
    let ex = dataset::export(
        &data,
        &ExportOptions {
            audio: AudioInclusion::Full,
        },
        "2026-09-24T00:00:00.000Z",
        &app(),
    );
    let key = format!(
        "audio/{}.wav",
        a.source_sha256().trim_start_matches("sha256:")
    );
    assert_eq!(nlae_core::hash::sha256(&ex.files[&key]), a.source_sha256());
}
