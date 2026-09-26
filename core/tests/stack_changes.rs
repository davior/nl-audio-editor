//! Apply-first editing (docs/spec/03-projects-clones-provenance.md, "Changing
//! the stack"): requests applied at once, steps excluded, restored, edited and
//! measured again, undone and redone. Every event matches the event schema,
//! the log verifies, and the dataset export says what became of each step.

use nlae_core::assistant::speech::{listen_params, DEFAULT_MODEL, PROVIDER};
use nlae_core::assistant::{record_dictation, Dictation, Segment};
use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::dataset::{self, ExportOptions, ProjectData};
use nlae_core::golden::{generate, spec_a};
use nlae_core::project::store::MemStore;
use nlae_core::project::*;
use nlae_core::provenance::{Actor, AppInfo, FixedEnv};
use nlae_core::scope::Scope;
use serde_json::{json, Value};

mod common;
use common::{schemas, validate};

fn app() -> AppInfo {
    AppInfo::new("test")
}

fn project(env: &mut FixedEnv) -> Project<MemStore> {
    let mut s = spec_a();
    s.duration_s = 8.0;
    s.long_pause = (5.0, 6.5);
    s.whine_spans = vec![(1.0, 2.0)];
    s.bang_times = vec![3.3];
    s.clip_span = (7.0, 7.3);
    let wav = write_wav(&generate(&s).mix, WavFormat::F32);
    let mut p = Project::create(
        MemStore::new(),
        env,
        app(),
        &wav,
        "interview.wav",
        CreateOptions::default(),
    )
    .unwrap();
    p.analyse(env).unwrap();
    p
}

fn draft(op: &str, params: Value) -> StepDraft {
    let mut d = StepDraft::new(op, params, Scope::Clip, Actor::user(), Origin::Console);
    d.intent = Some(format!("{op}, please"));
    d
}

fn apply(p: &mut Project<MemStore>, env: &mut FixedEnv, drafts: Vec<StepDraft>) -> Vec<Step> {
    p.apply(env, drafts, ApplyOptions::default()).unwrap()
}

/// Export the stack as it stands, as the front ends do.
fn export(p: &mut Project<MemStore>, env: &mut FixedEnv) {
    let fin = p.render_final().unwrap();
    p.record(
        env,
        "render.exported",
        None,
        json!({ "file": "out.wav", "format": "f32", "stack_hash": fin.stack_hash, "output_hash": fin.output_hash, "limiter": fin.limiter, "duration_s": fin.audio.duration_s(), "edits": fin.edits }),
    )
    .unwrap();
}

fn data(p: &mut Project<MemStore>) -> ProjectData {
    let src = p.source.clone();
    ProjectData {
        manifest: p.manifest.clone(),
        events: p.log.events().to_vec(),
        source_bytes: None,
        source_features: Some(p.features_for(&src)),
    }
}

#[test]
fn every_change_is_recorded_and_exported_with_what_became_of_each_step() {
    let (schemas, idx) = schemas();
    let mut env = FixedEnv::default();

    // --- A: a plan, a spoken request, a normalise; then pruned and tuned.
    let mut a = project(&mut env);
    let plan = apply(
        &mut a,
        &mut env,
        vec![draft("dc_remove", json!({})), draft("high_pass", json!({}))],
    );
    let heard = record_dictation(
        &mut a,
        &mut env,
        &Dictation {
            provider: PROVIDER.into(),
            model: DEFAULT_MODEL.into(),
            host: "api.deepgram.com".into(),
            params: listen_params(DEFAULT_MODEL, "en", 48_000, false).unwrap(),
            request_ids: vec!["req-1".into()],
            segments: vec![Segment {
                text: "quieter by six".into(),
                confidence: 0.9,
            }],
            words: "quieter by six dB".into(),
            audio_s: 1.5,
            latency_ms: 120,
        },
    )
    .unwrap();
    let gain = a
        .apply(
            &mut env,
            vec![draft("gain", json!({"gain_db": -6}))],
            ApplyOptions {
                dictation: Some(heard.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .remove(0);
    let norm = apply(&mut a, &mut env, vec![draft("normalise", json!({}))]).remove(0);
    let (hp, dc) = (&plan[1], &plan[0]);

    a.exclude(
        &mut env,
        std::slice::from_ref(&gain.step_id),
        Some("too quiet".into()),
        None,
    )
    .unwrap();
    a.edit(&mut env, &hp.step_id, &json!({"cutoff_hz": 150}), None)
        .unwrap();
    a.undo(&mut env, None).unwrap();
    a.redo(&mut env, None).unwrap();
    a.exclude(&mut env, std::slice::from_ref(&norm.step_id), None, None)
        .unwrap();
    a.restore(&mut env, std::slice::from_ref(&norm.step_id), None)
        .unwrap();
    assert!(
        a.state().entries[3].drifted,
        "measured with the gain in place"
    );
    a.remeasure(&mut env, &norm.step_id, None).unwrap();
    export(&mut a, &mut env);
    let (_, report) = Project::open(a.store.clone(), app()).unwrap();
    assert!(report.ok(), "{:?}", report.problems);

    // --- B: applied and exported, then changed again: kept, not approved.
    let mut b = project(&mut env);
    apply(&mut b, &mut env, vec![draft("dc_remove", json!({}))]);
    export(&mut b, &mut env);
    apply(&mut b, &mut env, vec![draft("normalise", json!({}))]);

    // --- Every event matches the event schema.
    for p in [&a, &b] {
        for ev in p.log.events() {
            validate(
                &schemas,
                idx["event.schema.json"],
                ev,
                &format!("{} event {}", p.id(), ev["type"]),
            );
        }
    }

    // --- The dataset export.
    let ex = dataset::export(
        &[data(&mut a), data(&mut b)],
        &ExportOptions::default(),
        "2026-09-26T00:00:00.000Z",
        &app(),
    );
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
    let episodes = lines("episodes.jsonl");
    for r in &episodes {
        validate(
            &schemas,
            idx["dataset/episode-record.schema.json"],
            r,
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

    let record = |id: &str| {
        steps
            .iter()
            .find(|r| r["id"].as_str().unwrap().ends_with(id))
            .unwrap()
    };
    let fate = |id: &str| record(id)["fate"].clone();

    // Every applied step has a record, as applied.
    for id in [&dc.step_id, &hp.step_id, &gain.step_id, &norm.step_id] {
        let r = record(id);
        assert_eq!(r["decision"]["outcome"], "applied");
        assert_eq!(r["decision"]["applied"], true);
        assert_eq!(r["outcome"]["measured_over"], "scope");
    }
    assert_eq!(
        record(&gain.step_id)["action"]["spoken"]["dictation"],
        heard
    );

    // Removed, with the reason.
    let g = fate(&gain.step_id);
    assert_eq!(g["outcome"], "removed");
    assert_eq!(g["removals"][0]["reason"], "too quiet");
    assert!(g.get("final").is_none());

    // Edited: the edit, its undo and its redo; the values it ended with.
    let h = fate(&hp.step_id);
    assert_eq!(h["outcome"], "approved");
    assert_eq!(h["edits"].as_array().unwrap().len(), 3);
    assert!(h["edits"][1]["undoes"].is_string() && h["edits"][2]["redoes"].is_string());
    assert_eq!(h["final"]["params"]["cutoff_hz"], 150);
    assert_eq!(record(&hp.step_id)["action"]["params"]["cutoff_hz"], 80);

    // Removed and restored, then measured again.
    let n = fate(&norm.step_id);
    assert_eq!(n["outcome"], "approved");
    assert_eq!(n["restorations"], 1);
    assert_eq!(n["removals"].as_array().unwrap().len(), 1);
    assert_eq!(n["edits"][0]["remeasured"], true);
    assert!(
        n["final"]["resolved"]["gain_db"] != record(&norm.step_id)["action"]["resolved"]["gain_db"]
    );
    assert_eq!(fate(&dc.step_id)["outcome"], "approved");
    assert_eq!(fate(&dc.step_id)["exported"], true);

    // B changed after its export: kept, though its first step was exported.
    let b_steps: Vec<&Value> = steps
        .iter()
        .filter(|r| r["project"]["id"] == b.id())
        .collect();
    assert!(b_steps.iter().all(|r| r["fate"]["outcome"] == "kept"));
    assert_eq!(b_steps[0]["fate"]["exported"], true);
    assert_eq!(b_steps[1]["fate"]["exported"], false);

    // Words become chat examples labelled with what became of them.
    let words = chat
        .iter()
        .find(|c| c["metadata"]["step_id"] == gain.step_id.as_str())
        .unwrap();
    assert_eq!(words["metadata"]["decision"], "removed");
    assert_eq!(words["metadata"]["fate"]["outcome"], "removed");

    // The episode is the stack at the end: active steps in their final versions.
    let ep = episodes
        .iter()
        .find(|e| e["project"]["id"] == a.id())
        .unwrap();
    let ops: Vec<&str> = ep["stack"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["op"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["dc_remove", "high_pass", "normalise"]);
    assert_eq!(ep["stack"][1]["params"]["cutoff_hz"], 150);
    assert_eq!(ep["counts"]["excluded"], 1);
    assert_eq!(ep["stack_hash"], json!(a.state().stack_hash));

    for (k, v) in [
        ("applied", 6u64),
        ("fate_removed", 1),
        ("fate_approved", 3),
        ("fate_kept", 2),
        ("fate_edited", 2),
    ] {
        assert_eq!(ex.counts.get(k).copied(), Some(v), "{k}: {:?}", ex.counts);
    }
}
