//! Labels (docs/spec/07-interface.md, "Labels"): short notes on a stretch of the
//! recording or on a time × frequency area. Their events match the event schema,
//! the log verifies, a bundle carries them, and they leave the stack alone.

use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::golden::{generate, spec_a};
use nlae_core::project::store::MemStore;
use nlae_core::project::*;
use nlae_core::provenance::{AppInfo, FixedEnv};
use serde_json::json;

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

fn place(t0: f64, t1: f64, band: Option<(f64, f64)>, text: &str) -> NewLabel {
    NewLabel {
        t0,
        t1,
        f_lo: band.map(|b| b.0),
        f_hi: band.map(|b| b.1),
        text: text.into(),
    }
}

#[test]
fn label_events_match_the_schema_and_travel_in_a_bundle() {
    let (schemas, idx) = schemas();
    let mut env = FixedEnv::default();
    let mut p = project(&mut env);

    // A label made on a time range and one made on an area, one edited, one removed.
    let cough = p
        .add_label(
            &mut env,
            place(1.2345678901234567, 1.75, None, "cough"),
            None,
        )
        .unwrap();
    let whine = p
        .add_label(
            &mut env,
            place(1.0, 2.0, Some((300.0, 900.0)), "whine"),
            None,
        )
        .unwrap();
    let gone = p
        .add_label(&mut env, place(3.2, 3.4, None, "mistake"), None)
        .unwrap();
    p.edit_label(&mut env, &cough.id, "cough, then a door", None)
        .unwrap();
    p.remove_label(&mut env, &gone.id, None).unwrap();
    let kept = p.labels();
    assert_eq!(
        kept.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        [cough.id.as_str(), whine.id.as_str()]
    );
    assert_eq!(
        kept[0].t0, 1.2345678901234567,
        "the place survives the log exactly"
    );

    // Every event of the log matches the event schema.
    let kinds = ["label.added", "label.edited", "label.removed"];
    let mut seen = 0;
    for ev in p.log.events() {
        validate(
            &schemas,
            idx["event.schema.json"],
            ev,
            &format!("event {}", ev["type"]),
        );
        seen += kinds.iter().filter(|k| ev["type"] == **k).count();
    }
    assert_eq!(seen, 5);

    // The schema refuses what the core would never write.
    let mut bad = p.log.events()[p.log.len() - 1].clone();
    bad["type"] = json!("label.added");
    bad["data"] = json!({ "label_id": "lb_x", "t0": 1.0, "t1": 2.0, "text": "two\nlines" });
    assert!(schemas.validate(&bad, idx["event.schema.json"]).is_err());
    bad["data"] =
        json!({ "label_id": "lb_x", "t0": 1.0, "t1": 2.0, "f_lo": 100.0, "text": "half a band" });
    assert!(schemas.validate(&bad, idx["event.schema.json"]).is_err());

    // The bundle carries them, and opening it verifies.
    let bytes = p.export_bundle(&mut env, None).unwrap();
    let (q, report) = open_bundle(&bytes, app()).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(q.labels(), kept);
    assert!(
        q.state().steps.is_empty(),
        "a label puts nothing on the stack"
    );
}
