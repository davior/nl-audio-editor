use serde_json::json;

use super::store::MemStore;
use super::*;
use crate::audio::wav::{write_wav, WavFormat};
use crate::golden;
use crate::provenance::FixedEnv;

fn short_clip() -> Vec<u8> {
    let mut s = golden::spec_a();
    s.duration_s = 8.0;
    s.long_pause = (5.0, 6.5);
    s.whine_spans = vec![(1.0, 2.0)];
    s.bang_times = vec![3.3];
    s.clip_span = (7.0, 7.3);
    write_wav(&golden::generate(&s).mix, WavFormat::F32)
}

fn app() -> AppInfo {
    AppInfo::new("test")
}

fn new_project(env: &mut FixedEnv) -> Project<MemStore> {
    Project::create(
        MemStore::new(),
        env,
        app(),
        &short_clip(),
        "field recording.wav",
        CreateOptions::default(),
    )
    .unwrap()
}

fn draft(op: &str, params: serde_json::Value, scope: Scope) -> StepDraft {
    StepDraft::new(op, params, scope, Actor::user(), Origin::Cli)
}

fn reopen(p: &Project<MemStore>) -> (Project<MemStore>, OpenReport) {
    Project::open(p.store.clone(), app()).unwrap()
}

#[test]
fn create_and_reopen_round_trips() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    p.set_view(json!({"zoom": 3, "selection": {"t0": 1.0, "t1": 2.0, "f_lo": 700, "f_hi": 800}}))
        .unwrap();
    let (q, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(
        q.source_bytes().unwrap(),
        short_clip(),
        "source stored byte-for-byte"
    );
    assert_eq!(q.manifest.view["selection"]["f_hi"], 800);
    assert_eq!(
        q.manifest.source.filename, "field recording.wav",
        "never renamed"
    );
    let kinds: Vec<&str> = q
        .log
        .events()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["project.created", "source.imported", "analysis.computed"]
    );
}

#[test]
fn preview_accept_and_the_log_rebuilds_the_stack() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let pv = p
        .preview(
            &mut env,
            vec![draft("dc_remove", json!({}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    assert_eq!(pv.record.kind, "step");
    assert_eq!(pv.output.len(), pv.before.len());
    assert!(p.state().steps.is_empty(), "nothing committed by a preview");
    let accepted = p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    assert_eq!(p.state().steps.len(), 1);
    let s = &accepted[0];
    assert!(s.state_before.is_some() && s.state_after.is_some() && s.output_hash.is_some());
    // The preview window equals the same span of the accepted full render.
    let full = p.current_render().unwrap();
    let (a, b) = (
        full.time_to_sample(pv.record.window[0]) as i64,
        full.time_to_sample(pv.record.window[1]) as i64,
    );
    assert_eq!(pv.output, full.extract_padded(a, b));
    // Reopen: the stack is rebuilt from the log and matches the manifest.
    let (q, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(q.state().steps, p.state().steps);
    assert_eq!(q.state().stack_hash, s.stack_hash.clone().unwrap());
    // Accepting twice is refused.
    assert!(p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .is_err());
}

#[test]
fn rejections_modifications_plans_and_undo_are_all_recorded() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let pv = p
        .preview(
            &mut env,
            vec![draft("gain", json!({"gain_db": 40}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    p.reject(
        &mut env,
        &pv.record.preview_id,
        Some("far too loud".into()),
        None,
    )
    .unwrap();
    assert!(p.state().steps.is_empty());

    let plan = vec![
        draft("dc_remove", json!({}), Scope::Clip),
        draft("normalise", json!({}), Scope::Clip),
        draft(
            "band_cut",
            json!({"f_lo": 740, "f_hi": 760, "depth_db": 12}),
            Scope::Clip,
        ),
    ];
    let pv = p
        .preview(&mut env, plan, PreviewOptions::default())
        .unwrap();
    assert_eq!(pv.record.kind, "plan");
    let mut opts = AcceptOptions::default();
    opts.overrides.insert(1, json!({"target_peak_dbfs": -3}));
    opts.disabled.push(2);
    let steps = p.accept(&mut env, &pv.record.preview_id, opts).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].params["target_peak_dbfs"], -3);
    assert!(steps.iter().all(|s| s.plan_id == pv.record.plan_id));
    let kinds: Vec<&str> = p
        .log
        .events()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"step.rejected")
            && kinds.contains(&"step.modified")
            && kinds.contains(&"plan.accepted")
    );

    let before_undo = p.state().stack_hash.clone();
    let removed = p.remove_top(&mut env, None).unwrap();
    assert_eq!(removed.op, "normalise");
    assert_ne!(p.state().stack_hash, before_undo);
    assert_eq!(
        p.state().stack_hash,
        p.state().steps[0].stack_hash.clone().unwrap()
    );
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn a_stale_preview_cannot_be_accepted() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let a = p
        .preview(
            &mut env,
            vec![draft("gain", json!({"gain_db": 3}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    let b = p
        .preview(
            &mut env,
            vec![draft("gain", json!({"gain_db": 6}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    p.accept(&mut env, &a.record.preview_id, AcceptOptions::default())
        .unwrap();
    let e = p
        .accept(&mut env, &b.record.preview_id, AcceptOptions::default())
        .unwrap_err();
    assert!(e.to_string().contains("changed since this preview"), "{e}");
}

#[test]
fn the_same_step_through_the_panel_and_the_console_differs_only_in_who_and_how() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let mut panel = draft(
        "band_cut",
        json!({"f_lo": 3100, "f_hi": 3200, "depth_db": 9}),
        Scope::Clip,
    );
    panel.origin = Origin::Panel;
    let mut console = panel.clone();
    console.origin = Origin::Console;
    console.actor = Actor::assistant("deepseek-chat", "deepseek");
    console.intent = Some("take the whine at about 3 kHz down".into());
    let a = p
        .preview(&mut env, vec![panel], PreviewOptions::default())
        .unwrap()
        .record
        .steps
        .remove(0);
    let b = p
        .preview(&mut env, vec![console], PreviewOptions::default())
        .unwrap()
        .record
        .steps
        .remove(0);
    assert_eq!(
        (&a.op, &a.params, &a.resolved, &a.scope),
        (&b.op, &b.params, &b.resolved, &b.scope)
    );
    assert_eq!(
        (&a.measurements, &a.input_hash, &a.state_before),
        (&b.measurements, &b.input_hash, &b.state_before)
    );
    assert_ne!(a.actor, b.actor);
    assert_ne!(a.origin, b.origin);
}

#[test]
fn accepted_manual_cuts_get_bindings_to_the_detected_line() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let pv = p
        .preview(
            &mut env,
            vec![draft(
                "band_cut",
                json!({"f_lo": 740, "f_hi": 760, "depth_db": 14}),
                Scope::Clip,
            )],
            PreviewOptions::default(),
        )
        .unwrap();
    let s = p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap()
        .remove(0);
    let b = &s.bindings["f_lo"];
    assert_eq!(b.feature, "tonal_lines");
    assert_eq!(b.source, BindingSource::Inferred);
    let near = b.select.as_ref().unwrap()["near_hz"].as_f64().unwrap();
    assert!((near - 750.0).abs() < 1.0, "{b:?}");
    assert!(s.bindings.contains_key("depth_db"));
}

#[test]
fn clones_verify_back_to_the_original_import() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let pv = p
        .preview(
            &mut env,
            vec![draft("dc_remove", json!({}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    let s1 = p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap()
        .remove(0);
    let pv = p
        .preview(
            &mut env,
            vec![draft("normalise", json!({}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();

    // Clone at the first step, then clone the clone.
    let mut c1 = p
        .clone_into(
            &mut env,
            MemStore::new(),
            Some(&s1.step_id),
            Some("stream B"),
            None,
        )
        .unwrap();
    assert_eq!(c1.state().steps.len(), 1);
    assert_eq!(
        c1.state().steps[0].inherited_from.as_ref().unwrap().step_id,
        s1.step_id
    );
    let pv = c1
        .preview(
            &mut env,
            vec![draft("gain", json!({"gain_db": 12}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    c1.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    let c2 = c1
        .clone_into(&mut env, MemStore::new(), None, None, None)
        .unwrap();
    assert_eq!(c2.state().steps.len(), 2);

    for proj in [&c1, &c2] {
        let (q, report) = Project::open(proj.store.clone(), app()).unwrap();
        assert!(report.ok(), "{:?}", report.problems);
        assert!(!report.lineage.is_empty());
        assert_eq!(q.source_bytes().unwrap(), p.source_bytes().unwrap());
    }
    let (_, report) = Project::open(c2.store.clone(), app()).unwrap();
    assert_eq!(report.lineage.len(), 2, "grandparent verified too");
    let kinds: Vec<&str> = p
        .log
        .events()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"project.clone_made"));

    // Tampering with an ancestor log inside the clone is detected.
    let mut store = c2.store.clone();
    let key = format!("lineage/{}.jsonl", p.id());
    let text = String::from_utf8(store.files[&key].clone())
        .unwrap()
        .replacen("\"project.created\"", "\"project.createD\"", 1);
    store.files.insert(key, text.into_bytes());
    let (_, report) = Project::open(store, app()).unwrap();
    assert!(!report.ok());
}

#[test]
fn bundles_round_trip_and_tampering_is_located() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    p.set_view(json!({"zoom": 2})).unwrap();
    let pv = p
        .preview(
            &mut env,
            vec![draft("dc_remove", json!({}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    let bytes = p.export_bundle(&mut env, None).unwrap();
    let (q, report) = open_bundle(&bytes, app()).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(q.source_bytes().unwrap(), short_clip());
    assert_eq!(q.manifest.view["zoom"], 2);
    assert_eq!(q.state().steps, p.state().steps);

    // Edit one event inside the bundle: the report names the line.
    let mut store = bundle::unpack(&bytes).unwrap();
    let text = String::from_utf8(store.files["events.jsonl"].clone()).unwrap();
    let edited = text.replacen("\"field recording\"", "\"field recordin9\"", 1);
    assert_ne!(edited, text);
    store
        .files
        .insert("events.jsonl".into(), edited.into_bytes());
    let (mut q, report) = Project::open(store, app()).unwrap();
    let f = report.log.first_failure.as_ref().expect("tamper detected");
    assert_eq!(f.line, 1);
    assert!(!report.ok());
    // Nothing is chained onto a broken log, and the view is not saved either.
    assert!(q.read_only().is_some());
    assert!(matches!(
        q.export_bundle(&mut env, None),
        Err(ProjectError::ReadOnly(_))
    ));
    assert!(matches!(
        q.set_view(json!({"zoom": 3})),
        Err(ProjectError::ReadOnly(_))
    ));
    assert!(q
        .clone_into(&mut env, MemStore::new(), None, None, None)
        .is_err());

    // Replacing the source is detected too.
    let mut store = bundle::unpack(&bytes).unwrap();
    let mut src = store.files["source/field recording.wav"].clone();
    let n = src.len();
    src[n - 1] ^= 1;
    store.files.insert("source/field recording.wav".into(), src);
    let (_, report) = Project::open(store, app()).unwrap();
    assert!(!report.source_ok);
}

#[test]
fn ratings_and_annotations_are_logged() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let pv = p
        .preview(
            &mut env,
            vec![draft("dc_remove", json!({}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    let s = p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap()
        .remove(0);
    p.annotate(
        &mut env,
        &s.step_id,
        Some("offset was large".into()),
        vec!["dc".into()],
        None,
    )
    .unwrap();
    let mut dims = BTreeMap::new();
    dims.insert("intelligibility".to_string(), 4u8);
    let stack = p.state().stack_hash.clone();
    p.rate(
        &mut env,
        Rating {
            target_kind: "stack".into(),
            target: stack,
            overall: 4,
            dims,
            note: None,
        },
        None,
    )
    .unwrap();
    assert!(p
        .rate(
            &mut env,
            Rating {
                target_kind: "stack".into(),
                target: "x".into(),
                overall: 6,
                dims: BTreeMap::new(),
                note: None
            },
            None
        )
        .is_err());
    assert_eq!(p.state().ratings.len(), 1);
    assert_eq!(p.state().annotations.len(), 1);
}

#[test]
fn the_final_render_ends_with_the_limiter() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let pv = p
        .preview(
            &mut env,
            vec![draft("gain", json!({"gain_db": 40}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    let f = p.render_final().unwrap();
    let peak = f.audio.channels[0]
        .iter()
        .fold(0.0f32, |m, &x| m.max(x.abs()));
    assert!(
        crate::math::amp_to_db(peak as f64) <= -0.99,
        "limited to −1 dBFS"
    );
    assert!(f.limiter["samples_limited"].as_u64().unwrap() > 0);
    // Steps cannot be the limiter.
    let e = p.preview(
        &mut env,
        vec![draft("limiter", json!({}), Scope::Clip)],
        PreviewOptions::default(),
    );
    assert!(e.is_err());
}
