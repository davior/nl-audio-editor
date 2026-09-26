//! Spoken requests (docs/spec/06-reasoning-layer.md): the transcript is logged
//! as `speech.transcribed` before it is acted on, the preview made from it
//! refers to it, the chain still verifies, and the dataset records say what
//! was heard and whether the user changed it.

use nlae_core::assistant::speech::{listen_params, DEFAULT_MODEL, PROVIDER};
use nlae_core::assistant::{record_dictation, route, Dictation, Route, Segment};
use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::dataset::{self, ExportOptions, ProjectData};
use nlae_core::golden::{generate, spec_a};
use nlae_core::project::store::MemStore;
use nlae_core::project::*;
use nlae_core::provenance::{Actor, AppInfo, FixedEnv};
use nlae_core::recipe::{preview_replay, Recipe, ReplayMode};
use serde_json::{json, Value};

mod common;
use common::{schemas, validate};

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
        AppInfo::new("test"),
        &wav,
        "call.wav",
        CreateOptions::default(),
    )
    .unwrap();
    p.analyse(env).unwrap();
    p
}

fn dictation(heard: &str, words: &str) -> Dictation {
    Dictation {
        provider: PROVIDER.into(),
        model: DEFAULT_MODEL.into(),
        host: "api.deepgram.com".into(),
        params: listen_params(DEFAULT_MODEL, "en", 48_000, false).unwrap(),
        request_ids: vec!["req-1".into()],
        segments: vec![Segment {
            text: heard.into(),
            confidence: 0.93,
        }],
        words: words.into(),
        audio_s: 2.1,
        latency_ms: 150,
    }
}

fn routed_drafts(words: &str) -> Vec<StepDraft> {
    match route(words, None) {
        Route::Steps { steps } => steps
            .iter()
            .map(|s| s.draft(Origin::Console, words))
            .collect(),
        other => panic!("expected steps for “{words}”, got {other:?}"),
    }
}

#[test]
fn a_spoken_request_is_logged_linked_to_its_preview_and_exported_with_what_was_heard() {
    let mut env = FixedEnv::default();
    let mut p = project(&mut env);
    let (schemas, idx) = schemas();

    // Heard "10 dB"; the user changed it to 12 before sending.
    let heard = "Cut 3100 to 3200 hertz by 10 dB.";
    let words = "Cut 3100 to 3200 hertz by 12 dB.";
    let h = record_dictation(&mut p, &mut env, &dictation(heard, words)).unwrap();
    let opts = PreviewOptions {
        dictation: Some(h.clone()),
        ..Default::default()
    };
    let pv = p.preview(&mut env, routed_drafts(words), opts).unwrap();
    assert_eq!(pv.record.dictation.as_deref(), Some(h.as_str()));
    assert_eq!(pv.record.steps[0].intent.as_deref(), Some(words));
    assert_eq!(pv.record.steps[0].params["depth_db"].as_f64(), Some(12.0));
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();

    // A recipe replayed from spoken words carries the link too.
    let words2 = "Clean this recording up.";
    let h2 = record_dictation(&mut p, &mut env, &dictation(words2, words2)).unwrap();
    let recipe = Recipe::builtin("spoken-word-cleanup").unwrap();
    let opts = PreviewOptions {
        dictation: Some(h2.clone()),
        ..Default::default()
    };
    let (_, pv2) = preview_replay(
        &mut p,
        &mut env,
        &recipe,
        ReplayMode::Adaptive,
        Actor::user(),
        opts,
        Some(words2),
    )
    .unwrap();
    assert_eq!(pv2.record.kind, "plan");
    assert!(pv2.record.recipe.is_some());
    assert_eq!(pv2.record.dictation.as_deref(), Some(h2.as_str()));
    p.reject(&mut env, &pv2.record.preview_id, None, None)
        .unwrap();

    // A link to anything but a logged spoken request is refused.
    let wrong = PreviewOptions {
        dictation: Some(pv.record.event_hash.clone()),
        ..Default::default()
    };
    assert!(p.preview(&mut env, routed_drafts(words), wrong).is_err());

    // What the log holds: the words heard and sent, the query, never a key.
    let events = p.log.events().to_vec();
    let spoken: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "speech.transcribed")
        .collect();
    assert_eq!(spoken.len(), 2);
    let d = &spoken[0]["data"];
    assert_eq!(d["heard"], heard);
    assert_eq!(d["words"], words);
    assert_eq!(d["edited"], true);
    assert_eq!(spoken[1]["data"]["edited"], false);
    assert_eq!(spoken[0]["actor"]["kind"], "user");
    assert!(d["params"]
        .as_array()
        .unwrap()
        .contains(&json!(["encoding", "linear16"])));
    for e in &events {
        validate(&schemas, idx["event.schema.json"], e, "event");
    }

    // The chain verifies after reopening, with the links intact.
    let (q, report) = Project::open(p.store.clone(), AppInfo::new("test")).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(
        q.state().previews[&pv.record.preview_id]
            .dictation
            .as_deref(),
        Some(h.as_str())
    );

    // The dataset says how each decided step was asked for.
    let src = p.source.clone();
    let data = [ProjectData {
        manifest: p.manifest.clone(),
        events,
        source_bytes: None,
        source_features: Some(p.features_for(&src)),
    }];
    let ex = dataset::export(
        &data,
        &ExportOptions::default(),
        "2026-09-25T00:00:00.000Z",
        &AppInfo::new("test"),
    );
    let lines = |f: &str| -> Vec<Value> {
        String::from_utf8(ex.files[f].clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    let steps = lines("steps.jsonl");
    let accepted = steps
        .iter()
        .find(|r| r["decision"]["outcome"] == "accepted")
        .unwrap();
    assert_eq!(
        accepted["action"]["spoken"],
        json!({ "dictation": h, "heard": heard, "edited": true, "provider": "deepgram", "model": "nova-3" })
    );
    assert_eq!(accepted["action"]["intent"], words);
    let rejected: Vec<&Value> = steps
        .iter()
        .filter(|r| r["decision"]["outcome"] == "rejected")
        .collect();
    assert!(!rejected.is_empty());
    assert!(rejected
        .iter()
        .all(|r| r["action"]["spoken"]["dictation"] == json!(h2)));
    for r in &steps {
        validate(
            &schemas,
            idx["dataset/step-record.schema.json"],
            r,
            "step record",
        );
    }
    let chat = lines("chat.jsonl");
    let c = chat
        .iter()
        .find(|c| c["metadata"]["decision"] == "accepted")
        .unwrap();
    assert_eq!(c["metadata"]["spoken"]["heard"], heard);
    for c in &chat {
        validate(
            &schemas,
            idx["dataset/chat-record.schema.json"],
            c,
            "chat record",
        );
    }
}

#[test]
fn a_dictation_is_refused_before_anything_is_logged() {
    let mut env = FixedEnv::default();
    let mut p = project(&mut env);
    let before = p.log.len();
    let mut d = dictation("undo", "undo");
    d.params.push(("key".into(), "dg-secret".into()));
    assert!(record_dictation(&mut p, &mut env, &d).is_err());
    let d = dictation("", "typed instead");
    assert!(record_dictation(&mut p, &mut env, &d).is_err());
    assert_eq!(p.log.len(), before, "nothing was logged");
}
