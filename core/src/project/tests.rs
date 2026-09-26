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
    let mut p = Project::create(
        MemStore::new(),
        env,
        app(),
        &short_clip(),
        "field recording.wav",
        CreateOptions::default(),
    )
    .unwrap();
    p.analyse(env).unwrap();
    p
}

#[test]
fn analysis_follows_creation_and_can_come_from_elsewhere() {
    let mut env = FixedEnv::default();
    let mut p = Project::create(
        MemStore::new(),
        &mut env,
        app(),
        &short_clip(),
        "field recording.wav",
        CreateOptions::default(),
    )
    .unwrap();
    assert!(p.needs_analysis(), "creating a project does not analyse it");
    assert_eq!(p.log.len(), 2);

    // Made elsewhere (a background worker): accepted only for this source.
    let f = crate::analysis::features(&p.source, None);
    let other = crate::analysis::features(
        &crate::audio::AudioBuffer::mono(48000, vec![0.1; 48000]),
        None,
    );
    let wrong = crate::audio::AudioBuffer::mono(48000, vec![0.1; 48000]).render_hash();
    assert!(p.record_analysis(&mut env, other, &wrong).is_err());
    let mut old = f.clone();
    old.version = 1;
    let hash = p.source_render_hash().to_string();
    assert!(p.record_analysis(&mut env, old, &hash).is_err());
    p.record_analysis(&mut env, f.clone(), &hash).unwrap();
    assert!(!p.needs_analysis());
    assert_eq!(p.log.len(), 3);
    // Once only.
    p.record_analysis(&mut env, f.clone(), &hash).unwrap();
    p.analyse(&mut env).unwrap();
    assert_eq!(p.log.len(), 3);

    // The same as analysing here.
    let mut q = new_project(&mut FixedEnv::default());
    let a = &p.log.events()[2]["data"];
    let b = &q.log.events()[2]["data"];
    assert_eq!(a["features_hash"], b["features_hash"]);
    assert_eq!(a["render_hash"], b["render_hash"]);
    assert!(!q.needs_analysis());
    q.analyse(&mut FixedEnv::default()).unwrap();
    assert_eq!(q.log.len(), 3);
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

#[test]
fn borrowed_renders_are_the_renders() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let source = p.source.clone();
    assert_eq!(p.audio("stack").unwrap(), &source);
    let pv = p
        .preview(
            &mut env,
            vec![draft("line_reduce", json!({}), Scope::Clip)],
            PreviewOptions::default(),
        )
        .unwrap();
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    let stack = p.current_render().unwrap();
    let residual = p.residual_render().unwrap();
    assert_eq!(p.audio("stack").unwrap(), &stack);
    assert_eq!(p.audio("residual").unwrap(), &residual);
    assert_eq!(p.audio("source").unwrap(), &source);
    assert!(p.audio("other").is_err());
}

fn preview_one(p: &mut Project<MemStore>, env: &mut FixedEnv, d: StepDraft) -> Preview {
    p.preview(env, vec![d], PreviewOptions::default()).unwrap()
}

#[test]
fn time_edits_shape_the_output_and_leave_the_processing_alone() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let sr = p.source.sample_rate as usize;
    let len = p.source.len();
    let fade = sr * 5 / 1000;
    let processed = p.current_render().unwrap();

    // Remove 2–3 s: the preview shows the join, and what goes, where it was.
    let pv = preview_one(
        &mut p,
        &mut env,
        draft(
            "remove_time",
            json!({}),
            Scope::TimeRange { t0: 2.0, t1: 3.0 },
        ),
    );
    assert_eq!(pv.record.window, [0.0, 6.0]);
    assert_eq!(pv.before.len() - pv.output.len(), sr);
    let r = &pv.residual.channels[0];
    assert!(r[..2 * sr].iter().all(|&x| x == 0.0) && r[3 * sr..].iter().all(|&x| x == 0.0));
    assert_eq!(&r[2 * sr..3 * sr], &pv.before.channels[0][2 * sr..3 * sr]);
    let m = &pv.record.steps[0].measurements;
    assert_eq!(m["removed_s"], 1.0);
    assert_eq!(
        m["output_duration_s"].as_f64(),
        Some((len - sr) as f64 / sr as f64)
    );
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();

    // Insert 0.5 s of silence at 6 s of the original.
    let pv = preview_one(
        &mut p,
        &mut env,
        draft(
            "insert_silence",
            json!({ "at_s": 6.0, "duration_s": 0.5 }),
            Scope::Clip,
        ),
    );
    assert_eq!(pv.output.len() - pv.before.len(), sr / 2);
    p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    assert_eq!(p.state().steps[1].class, crate::ops::OpClass::Edit);

    // Processing never sees the edits.
    assert_eq!(
        p.current_render().unwrap().render_hash(),
        processed.render_hash()
    );
    // The output: the original up to the fade before the join, exactly; after
    // it, the original from 3 s on; then the silence where 6 s of the original was.
    let out = p.audio("output").unwrap().clone();
    assert_eq!(out.len(), len - sr + sr / 2);
    let (o, src) = (&out.channels[0], &processed.channels[0]);
    assert_eq!(&o[..2 * sr - fade], &src[..2 * sr - fade]);
    assert_eq!(
        &o[2 * sr + fade..5 * sr - fade],
        &src[3 * sr + fade..6 * sr - fade]
    );
    assert!(o[5 * sr..5 * sr + sr / 2].iter().all(|&x| x == 0.0));
    assert_eq!(&o[5 * sr + sr / 2 + fade..], &src[6 * sr + fade..]);

    // The export ends with the limiter and says where the edits are.
    let fin = p.render_final().unwrap();
    assert_eq!(fin.audio.len(), out.len());
    assert_eq!(
        fin.cues,
        vec![
            (
                2 * sr as u32,
                "removed 2.000-3.000 s of the original (1.000 s)".to_string()
            ),
            (
                5 * sr as u32,
                "inserted 0.500 s of silence at 6.000 s of the original".to_string()
            ),
        ]
    );
    assert_eq!(fin.edits[1]["kind"], "inserted");
    assert_eq!(fin.edits[1]["at_output_s"], 5.0);

    // Reopened, the log verifies and gives the same output.
    let (mut q, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(q.edit_layout(), p.edit_layout());
    assert_eq!(q.render_final().unwrap().output_hash, fin.output_hash);

    // Undo takes the silence out again; recipes leave time edits out.
    p.remove_top(&mut env, None).unwrap();
    assert_eq!(p.render_final().unwrap().audio.len(), len - sr);
    assert!(crate::recipe::Recipe::from_project(&p, "cut", 0, 0).is_err());
}

#[test]
fn a_time_edit_is_previewed_alone_and_inside_the_recording() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let plan = vec![
        draft("gain", json!({ "gain_db": 3 }), Scope::Clip),
        draft(
            "remove_time",
            json!({}),
            Scope::TimeRange { t0: 1.0, t1: 2.0 },
        ),
    ];
    let e = p
        .preview(&mut env, plan, PreviewOptions::default())
        .err()
        .unwrap();
    assert!(e.to_string().contains("on its own"), "{e}");
    let late = draft(
        "insert_silence",
        json!({ "at_s": 100.0, "duration_s": 1.0 }),
        Scope::Clip,
    );
    let e = p
        .preview(&mut env, vec![late], PreviewOptions::default())
        .err()
        .unwrap();
    assert!(e.to_string().contains("after the end"), "{e}");
    let missing = draft("insert_silence", json!({ "duration_s": 1.0 }), Scope::Clip);
    let e = p
        .preview(&mut env, vec![missing], PreviewOptions::default())
        .err()
        .unwrap();
    assert!(e.to_string().contains("at_s"), "{e}");
}

// --- Changing the stack: apply, exclude, restore, edit, undo, redo ---

fn apply_one(p: &mut Project<MemStore>, env: &mut FixedEnv, d: StepDraft) -> Step {
    p.apply(env, vec![d], ApplyOptions::default())
        .unwrap()
        .remove(0)
}

fn ids(steps: &[Step]) -> Vec<String> {
    steps.iter().map(|s| s.step_id.clone()).collect()
}

fn peak_db(a: &AudioBuffer) -> f64 {
    let peak = a
        .channels
        .iter()
        .flatten()
        .fold(0.0f32, |m, &x| m.max(x.abs()));
    crate::math::amp_to_db(peak as f64)
}

#[test]
fn applying_records_what_accepting_a_preview_records() {
    let d = draft(
        "band_cut",
        json!({"f_lo": 740, "f_hi": 760, "depth_db": 14}),
        Scope::Clip,
    );
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let applied = p
        .apply(&mut env, vec![d.clone()], ApplyOptions::default())
        .unwrap()
        .remove(0);
    let mut env_b = FixedEnv::default();
    let mut q = new_project(&mut env_b);
    let pv = q
        .preview(&mut env_b, vec![d], PreviewOptions::default())
        .unwrap();
    let accepted = q
        .accept(&mut env_b, &pv.record.preview_id, AcceptOptions::default())
        .unwrap()
        .remove(0);
    let what = |s: &Step| {
        (
            s.resolved.clone(),
            s.measurements.clone(),
            s.output_hash.clone(),
            s.stack_hash.clone(),
            s.state_before.clone(),
            s.state_after.clone(),
            s.bindings.clone(),
        )
    };
    assert_eq!(what(&applied), what(&accepted));
    let ev = p.log.events().last().unwrap();
    assert_eq!(ev["type"], "step.applied");
    assert_eq!(ev["data"]["kind"], "step");
    assert_eq!(p.state().undo.len(), 1);
    assert_eq!(p.state().undo[0].kind, "applied");

    // Several steps are one plan; a spoken request named must be in the log.
    let plan = p
        .apply(
            &mut env,
            vec![
                draft("dc_remove", json!({}), Scope::Clip),
                draft("normalise", json!({}), Scope::Clip),
            ],
            ApplyOptions::default(),
        )
        .unwrap();
    assert!(plan[0].plan_id.is_some() && plan[0].plan_id == plan[1].plan_id);
    assert_eq!(p.log.events().last().unwrap()["data"]["kind"], "plan");
    let e = p
        .apply(
            &mut env,
            vec![draft("gain", json!({"gain_db": 1}), Scope::Clip)],
            ApplyOptions {
                dictation: Some("sha256:00".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(e.to_string().contains("no spoken request"), "{e}");
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn a_middle_step_is_excluded_and_the_steps_above_keep_their_values() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let dc = apply_one(&mut p, &mut env, draft("dc_remove", json!({}), Scope::Clip));
    let gain = apply_one(
        &mut p,
        &mut env,
        draft("gain", json!({"gain_db": -6}), Scope::Clip),
    );
    let norm = apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    let chain = p
        .exclude(
            &mut env,
            std::slice::from_ref(&gain.step_id),
            Some("too quiet".into()),
            None,
        )
        .unwrap();

    // The steps above are recorded again, with their values.
    assert_eq!(ids(&chain), std::slice::from_ref(&norm.step_id));
    let now = p.state().steps.clone();
    assert_eq!(ids(&now), [dc.step_id.clone(), norm.step_id.clone()]);
    assert_eq!(
        now[1].resolved, norm.resolved,
        "the normalise keeps its gain"
    );
    assert_ne!(now[1].stack_hash, norm.stack_hash);
    assert_ne!(now[1].output_hash, norm.output_hash);
    // Its gain was measured on audio that is no longer its input.
    assert_ne!(now[1].input_hash, norm.input_hash);
    assert_eq!(
        now[1].resolved_on.as_deref(),
        Some(norm.input_hash.as_str())
    );

    // The excluded step keeps its place.
    let e = &p.state().entries;
    assert_eq!(e.len(), 3);
    assert!(e[0].active && !e[1].active && e[2].active);
    assert_eq!(e[1].reason.as_deref(), Some("too quiet"));
    assert!(e[2].drifted && !e[0].drifted);
    assert_eq!(p.state().excluded[&gain.step_id].resolved, gain.resolved);

    // What renders is exactly the active steps.
    let render = crate::engine::render_full(&p.source, &p.render_steps())
        .unwrap()
        .audio;
    assert_eq!(p.current_render().unwrap(), render);
    assert_eq!(Some(render.render_hash()), now[1].output_hash);
    assert!((peak_db(&render) - 5.0).abs() < 0.01, "6 dB hotter than −1");

    // The log rebuilds all of it.
    let (q, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(q.state(), p.state());

    // An excluded step is not excluded twice, nor edited.
    let again = p.exclude(&mut env, std::slice::from_ref(&gain.step_id), None, None);
    assert!(again.unwrap_err().to_string().contains("already excluded"));
    let edit = p.edit(&mut env, &gain.step_id, &json!({"gain_db": -3}), None);
    assert!(edit.unwrap_err().to_string().contains("restore it first"));
}

#[test]
fn restoring_brings_back_the_same_stack_from_the_cache() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    apply_one(&mut p, &mut env, draft("dc_remove", json!({}), Scope::Clip));
    let gain = apply_one(
        &mut p,
        &mut env,
        draft("gain", json!({"gain_db": -6}), Scope::Clip),
    );
    let norm = apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    let before = p.state().clone();
    let (rendered, _) = p.renders.get(&before.stack_hash).unwrap();

    p.exclude(&mut env, std::slice::from_ref(&gain.step_id), None, None)
        .unwrap();
    let restored = p
        .restore(&mut env, std::slice::from_ref(&gain.step_id), None)
        .unwrap();
    assert_eq!(ids(&restored), [gain.step_id.clone(), norm.step_id.clone()]);
    assert_eq!(p.state().steps, before.steps, "every field as it was");
    assert_eq!(p.state().stack_hash, before.stack_hash);
    assert!(p
        .state()
        .entries
        .iter()
        .all(|e| e.active && !e.drifted && e.reason.is_none()));
    // Not rendered again: the render made before is the one kept.
    let (again, _) = p.renders.get(&before.stack_hash).unwrap();
    assert!(Arc::ptr_eq(&rendered, &again));
    // Only an excluded step can be restored.
    let e = p.restore(&mut env, std::slice::from_ref(&gain.step_id), None);
    assert!(e.unwrap_err().to_string().contains("already on the stack"));
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn measuring_again_brings_a_drifted_step_back_to_its_target() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let gain = apply_one(
        &mut p,
        &mut env,
        draft("gain", json!({"gain_db": -6}), Scope::Clip),
    );
    let norm = apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    p.exclude(&mut env, std::slice::from_ref(&gain.step_id), None, None)
        .unwrap();
    assert!((peak_db(&p.current_render().unwrap()) - 5.0).abs() < 0.01);

    // What would change is shown first; nothing is changed or logged.
    let n = p.log.len();
    let diff = p.remeasure_diff(&norm.step_id).unwrap();
    assert_eq!(p.log.len(), n);
    let g = diff
        .iter()
        .find(|d| d.param == "resolved.gain_db")
        .expect("the gain changes");
    let was = norm.resolved["gain_db"].as_f64().unwrap();
    assert!(
        (g.new.as_f64().unwrap() - (was - 6.0)).abs() < 0.01,
        "{g:?}"
    );

    p.remeasure(&mut env, &norm.step_id, None).unwrap();
    let s = p.state().steps.last().unwrap().clone();
    assert_eq!(s.params, norm.params);
    assert!(s.resolved_on.is_none() && !p.state().entries[1].drifted);
    assert!((peak_db(&p.current_render().unwrap()) + 1.0).abs() < 1e-3);
    let ev = p.log.events().last().unwrap();
    assert_eq!(ev["type"], "step.edited");
    assert_eq!(ev["data"]["remeasured"], true);
    assert_eq!(p.state().entries[1].edits, 1);
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn an_edit_resolves_the_step_again_and_the_steps_above_keep_their_values() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let hp = apply_one(&mut p, &mut env, draft("high_pass", json!({}), Scope::Clip));
    let norm = apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    let chain = p
        .edit(&mut env, &hp.step_id, &json!({"cutoff_hz": 150}), None)
        .unwrap();
    assert_eq!(ids(&chain), [hp.step_id.clone(), norm.step_id.clone()]);
    let now = p.state().steps.clone();
    assert_eq!(now[0].step_id, hp.step_id, "the same step, a new version");
    assert_eq!(now[0].params["cutoff_hz"], 150.0);
    assert_eq!(
        now[0].params["slope_db_per_octave"],
        hp.params["slope_db_per_octave"]
    );
    assert!(now[0].resolved_on.is_none());
    assert_eq!(now[1].resolved, norm.resolved);
    assert!(now[1].resolved_on.is_some());
    assert_eq!(p.state().entries[0].edits, 1);

    // The edited step is what a 150 Hz high-pass applied afresh would be.
    let mut env_b = FixedEnv::default();
    let mut q = new_project(&mut env_b);
    let fresh = apply_one(
        &mut q,
        &mut env_b,
        draft("high_pass", json!({"cutoff_hz": 150}), Scope::Clip),
    );
    assert_eq!(
        (&now[0].resolved, &now[0].output_hash, &now[0].measurements),
        (&fresh.resolved, &fresh.output_hash, &fresh.measurements)
    );
    let render = crate::engine::render_full(&p.source, &p.render_steps())
        .unwrap()
        .audio;
    assert_eq!(Some(render.render_hash()), now[1].output_hash);

    // Out of range is rejected, not clamped; no change is no edit.
    let n = p.log.len();
    assert!(p
        .edit(&mut env, &hp.step_id, &json!({"cutoff_hz": 1e9}), None)
        .is_err());
    let same = p.edit(&mut env, &hp.step_id, &json!({"cutoff_hz": 150}), None);
    assert!(same.unwrap_err().to_string().contains("unchanged"));
    assert!(p.edit(&mut env, "st_none", &json!({}), None).is_err());
    assert_eq!(p.log.len(), n);
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn undo_and_redo_walk_back_through_the_changes() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let hash = |p: &Project<MemStore>| p.state().stack_hash.clone();
    let kinds = |p: &Project<MemStore>| {
        p.state()
            .undo
            .iter()
            .map(|c| c.kind.clone())
            .collect::<Vec<_>>()
    };
    let s0 = hash(&p);
    let dc = apply_one(&mut p, &mut env, draft("dc_remove", json!({}), Scope::Clip));
    let s1 = hash(&p);
    let plan = p
        .apply(
            &mut env,
            vec![
                draft("gain", json!({"gain_db": 3}), Scope::Clip),
                draft(
                    "band_cut",
                    json!({"f_lo": 740, "f_hi": 760, "depth_db": 12}),
                    Scope::Clip,
                ),
            ],
            ApplyOptions::default(),
        )
        .unwrap();
    let s2 = hash(&p);
    p.exclude(&mut env, std::slice::from_ref(&dc.step_id), None, None)
        .unwrap();
    let s3 = hash(&p);
    p.edit(&mut env, &plan[0].step_id, &json!({"gain_db": 6}), None)
        .unwrap();
    assert_eq!(kinds(&p), ["applied", "applied", "excluded", "edited"]);

    // Back, one change at a time, to exactly where each change started.
    assert_eq!(p.undo(&mut env, None).unwrap().kind, "edited");
    assert_eq!(hash(&p), s3);
    assert_eq!(
        p.state().step(&plan[0].step_id).unwrap().params["gain_db"],
        3.0
    );
    assert_eq!(p.state().entries[1].edits, 0);
    assert_eq!(p.undo(&mut env, None).unwrap().kind, "excluded");
    assert_eq!(hash(&p), s2);
    let undone = p.undo(&mut env, None).unwrap();
    assert_eq!(undone.kind, "applied");
    assert_eq!(undone.step_ids, ids(&plan), "the plan comes off as one");
    assert_eq!(hash(&p), s1);
    assert_eq!(p.state().steps.len(), 1);

    // And forward again.
    assert_eq!(p.redo(&mut env, None).unwrap().kind, "applied");
    assert_eq!(hash(&p), s2);
    assert_eq!(p.redo(&mut env, None).unwrap().kind, "excluded");
    assert_eq!(hash(&p), s3);
    assert_eq!(p.state().redo.len(), 1);

    // Each undo and redo is logged, and a reload keeps both lists.
    let undos = p
        .log
        .events()
        .iter()
        .filter(|e| e["data"]["undoes"].is_string())
        .count();
    assert_eq!(undos, 3);
    let (q, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(
        (&q.state().undo, &q.state().redo),
        (&p.state().undo, &p.state().redo)
    );

    // A new change clears what could be redone.
    p.edit(&mut env, &plan[1].step_id, &json!({"depth_db": 6}), None)
        .unwrap();
    assert!(p.state().redo.is_empty());
    assert!(p.redo(&mut env, None).is_err());

    // All the way back: every step is excluded, and still in its place.
    while !p.state().undo.is_empty() {
        p.undo(&mut env, None).unwrap();
    }
    assert_eq!(hash(&p), s0);
    assert!(p.state().steps.is_empty());
    assert_eq!(p.state().entries.len(), 3);
    assert!(p.undo(&mut env, None).is_err());
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn a_change_that_does_not_fit_the_stack_is_refused_and_never_written() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let gain = apply_one(
        &mut p,
        &mut env,
        draft("gain", json!({"gain_db": -6}), Scope::Clip),
    );
    let norm = apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    let n = p.log.len();
    let written = p.store.files["events.jsonl"].clone();

    // A chain that changes a value the change leaves alone.
    let mut changed = norm.clone();
    changed.resolved["gain_db"] = json!(0.0);
    let e = p
        .append(
            &mut env,
            "step.excluded",
            &Actor::user(),
            json!({ "step_ids": [gain.step_id], "chain": [changed] }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("changes the values"), "{e}");
    // A chain that leaves out a step above the change.
    let e = p
        .append(
            &mut env,
            "step.excluded",
            &Actor::user(),
            json!({ "step_ids": [gain.step_id], "chain": [] }),
        )
        .unwrap_err();
    assert!(
        e.to_string().contains("active steps from the change up"),
        "{e}"
    );
    // An undo of something that is not the last change.
    let e = p
        .append(
            &mut env,
            "step.excluded",
            &Actor::user(),
            json!({ "step_ids": [norm.step_id], "chain": [], "undoes": "sha256:00" }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("not the last change"), "{e}");
    assert_eq!(p.log.len(), n);
    assert_eq!(p.store.files["events.jsonl"], written, "nothing written");
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn a_rehashed_log_whose_chain_changes_a_value_does_not_verify() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let gain = apply_one(
        &mut p,
        &mut env,
        draft("gain", json!({"gain_db": -6}), Scope::Clip),
    );
    apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    p.exclude(&mut env, std::slice::from_ref(&gain.step_id), None, None)
        .unwrap();

    // Give the normalise another gain in the exclusion, and re-hash every event
    // so that the hash chain itself still holds.
    let mut events: Vec<serde_json::Value> = p.log.events().to_vec();
    events.last_mut().unwrap()["data"]["chain"][0]["resolved"]["gain_db"] = json!(0.0);
    for i in 0..events.len() {
        if i > 0 {
            events[i]["prev"] = events[i - 1]["hash"].clone();
        }
        events[i].as_object_mut().unwrap().remove("hash");
        let h = crate::provenance::log::event_hash(&events[i]).unwrap();
        events[i]["hash"] = json!(h);
    }
    let mut store = p.store.clone();
    let text: String = events.iter().map(EventLog::line_of).collect();
    store.files.insert("events.jsonl".into(), text.into_bytes());
    let mut m = p.manifest.clone();
    m.log.head = events.last().unwrap()["hash"].as_str().unwrap().into();
    store
        .files
        .insert("manifest.json".into(), serde_json::to_vec(&m).unwrap());

    let (q, report) = Project::open(store, app()).unwrap();
    assert!(report.log.ok, "the hash chain holds");
    assert!(!report.projection_ok);
    assert!(q.read_only().is_some());
}

#[test]
fn removals_written_before_can_be_undone_and_restored() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    for d in [
        draft("dc_remove", json!({}), Scope::Clip),
        draft("gain", json!({"gain_db": 3}), Scope::Clip),
    ] {
        let pv = preview_one(&mut p, &mut env, d);
        p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
            .unwrap();
    }
    let gain = p.state().steps[1].clone();
    // As undoing the top step was logged before steps could be excluded.
    p.append(
        &mut env,
        "step.removed",
        &Actor::user(),
        json!({ "step_id": gain.step_id }),
    )
    .unwrap();
    assert_eq!(p.state().steps.len(), 1);
    assert!(!p.state().entries[1].active);
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);

    // Undo takes the removal back.
    let c = p.undo(&mut env, None).unwrap();
    assert_eq!(c.kind, "excluded");
    assert_eq!(p.state().steps[1], gain);
    let ev = p.log.events().last().unwrap();
    assert_eq!(ev["type"], "step.restored");
    assert_eq!(ev["data"]["undoes"], json!(c.event));
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn excluding_a_time_edit_renders_nothing_again() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let cut = apply_one(
        &mut p,
        &mut env,
        draft(
            "remove_time",
            json!({}),
            Scope::TimeRange { t0: 2.0, t1: 3.0 },
        ),
    );
    let norm = apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    let (rendered, _) = p.renders.get(norm.stack_hash.as_ref().unwrap()).unwrap();
    p.exclude(&mut env, std::slice::from_ref(&cut.step_id), None, None)
        .unwrap();
    let now = p.state().steps[0].clone();
    assert_eq!(now.output_hash, norm.output_hash, "the same audio");
    assert_eq!(now.measurements, norm.measurements);
    assert!(now.resolved_on.is_none(), "its input did not change");
    let (again, _) = p.renders.get(now.stack_hash.as_ref().unwrap()).unwrap();
    assert!(Arc::ptr_eq(&rendered, &again), "not rendered again");
    assert_eq!(p.render_final().unwrap().audio.len(), p.source.len());
    let (_, report) = reopen(&p);
    assert!(report.ok(), "{:?}", report.problems);
}

#[test]
fn a_clone_edits_and_undoes_what_it_inherited() {
    let mut env = FixedEnv::default();
    let mut p = new_project(&mut env);
    let gain = apply_one(
        &mut p,
        &mut env,
        draft("gain", json!({"gain_db": 3}), Scope::Clip),
    );
    apply_one(&mut p, &mut env, draft("normalise", json!({}), Scope::Clip));
    let mut c = p
        .clone_into(&mut env, MemStore::new(), None, None, None)
        .unwrap();
    c.edit(&mut env, &gain.step_id, &json!({"gain_db": 6}), None)
        .unwrap();
    c.undo(&mut env, None).unwrap();
    assert_eq!(c.state().steps[0].params, gain.params);
    assert_eq!(c.state().stack_hash, p.state().stack_hash);
    let (_, report) = Project::open(c.store.clone(), app()).unwrap();
    assert!(report.ok(), "{:?}", report.problems);

    // Excluded steps are not inherited, and a clone cannot start at one.
    p.exclude(&mut env, std::slice::from_ref(&gain.step_id), None, None)
        .unwrap();
    assert!(p
        .clone_into(&mut env, MemStore::new(), Some(&gain.step_id), None, None)
        .is_err());
    let c2 = p
        .clone_into(&mut env, MemStore::new(), None, None, None)
        .unwrap();
    assert_eq!(c2.state().steps.len(), 1);
    let (_, report) = Project::open(c2.store.clone(), app()).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
}
