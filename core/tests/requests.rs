//! The requests made of a project, rebuilt from its log (`assistant::requests`):
//! each with its answer and the steps it put on the stack, in the order they
//! were made, and the same after the project is reopened. The browser shows
//! them as one list with the stack.

use nlae_core::assistant::speech::{listen_params, DEFAULT_MODEL, PROVIDER};
use nlae_core::assistant::{
    next_round, record_dictation, record_exchange, request_for, requests, route, Dictation,
    Exchange, LoggedRequest, NextRound, Proposal, Rounds, Route, Segment, Via, DESCRIBE_TOOL,
};
use nlae_core::audio::wav::{write_wav, WavFormat};
use nlae_core::golden::{generate, spec_a};
use nlae_core::project::store::MemStore;
use nlae_core::project::*;
use nlae_core::provenance::{Actor, AppInfo, FixedEnv};
use nlae_core::recipe::{apply_replay, Recipe, ReplayMode};
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

fn spoken(p: &mut Project<MemStore>, env: &mut FixedEnv, words: &str) -> String {
    let d = Dictation {
        provider: PROVIDER.into(),
        model: DEFAULT_MODEL.into(),
        host: "api.deepgram.com".into(),
        params: listen_params(DEFAULT_MODEL, "en", 48_000, false).unwrap(),
        request_ids: vec!["req-1".into()],
        segments: vec![Segment {
            text: words.into(),
            confidence: 0.9,
        }],
        words: words.into(),
        audio_s: 1.5,
        latency_ms: 120,
    };
    record_dictation(p, env, &d).unwrap()
}

/// A model's answer: its words, and the tools it called.
fn answer(content: Option<&str>, calls: &[(&str, Value)]) -> Value {
    let mut message = json!({ "role": "assistant", "content": content });
    if !calls.is_empty() {
        let calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| {
                json!({ "id": format!("call_{i}"), "type": "function",
                    "function": { "name": name, "arguments": args.to_string() } })
            })
            .collect();
        message["tool_calls"] = json!(calls);
    }
    json!({ "choices": [{ "message": message }] })
}

fn gain(db: f64) -> (&'static str, Value) {
    (
        "gain",
        json!({ "params": { "gain_db": db }, "scope": { "kind": "clip" } }),
    )
}

/// Ask the model as the front ends do, its answers given in order: every
/// exchange is logged, linked to the one before and to the dictation. Returns
/// the last exchange and what it proposed, if it could be used.
fn ask(
    p: &mut Project<MemStore>,
    env: &mut FixedEnv,
    words: &str,
    answers: &[Value],
    dictation: Option<&str>,
) -> (String, Option<Proposal>) {
    let mut request = request_for(p, "m-1", &[], words, None).unwrap();
    let mut rounds = Rounds::default();
    let (mut corrects, mut describes) = (None, None);
    for a in answers {
        let next = next_round(&request, a, rounds).unwrap();
        let ex = Exchange {
            provider: "mock".into(),
            model: "m-1".into(),
            host: "example.test".into(),
            request: request.to_string(),
            response: a.clone(),
            latency_ms: 5,
            problems: next.problems(),
            corrects: corrects.take(),
            describes: describes.take(),
            dictation: dictation.map(str::to_string),
        };
        let h = record_exchange(p, env, &ex).unwrap();
        match next {
            NextRound::Done { proposal } => return (h, Some(proposal)),
            NextRound::Failed { .. } => return (h, None),
            NextRound::Correct { request: r, .. } => {
                rounds.corrected = true;
                corrects = Some(h);
                request = r;
            }
            NextRound::Describe { request: r, .. } => {
                rounds.described = true;
                describes = Some(h);
                request = r;
            }
        }
    }
    panic!("the answers ran out");
}

fn apply_proposal(
    p: &mut Project<MemStore>,
    env: &mut FixedEnv,
    words: &str,
    proposal: &Proposal,
    exchange: &str,
) -> Vec<Step> {
    let drafts = proposal.drafts(&Actor::assistant("m-1", "mock"), Origin::Console, words);
    let opts = ApplyOptions {
        plan: proposal.plan,
        exchange: Some(exchange.to_string()),
        ..Default::default()
    };
    p.apply(env, drafts, opts).unwrap()
}

fn apply_routed(
    p: &mut Project<MemStore>,
    env: &mut FixedEnv,
    words: &str,
    dictation: Option<String>,
) -> (Vec<Step>, String) {
    let Route::Steps { steps } = route(words, None) else {
        panic!("expected “{words}” to be routed to steps");
    };
    let understood = steps
        .iter()
        .map(|s| s.understood.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let drafts = steps
        .iter()
        .map(|s| s.draft(Origin::Console, words))
        .collect();
    let opts = ApplyOptions {
        dictation,
        ..Default::default()
    };
    (p.apply(env, drafts, opts).unwrap(), understood)
}

fn seq_of(p: &Project<MemStore>, hash: &str) -> u64 {
    p.log
        .events()
        .iter()
        .find(|e| e["hash"] == hash)
        .and_then(|e| e["seq"].as_u64())
        .unwrap()
}

fn ids(steps: &[Step]) -> Vec<String> {
    steps.iter().map(|s| s.step_id.clone()).collect()
}

#[test]
fn every_request_comes_back_from_the_log_with_its_answer_and_its_steps() {
    let mut env = FixedEnv::default();
    let mut p = project(&mut env);
    let (schemas, idx) = schemas();

    // Routed without a model.
    let (cut, understood) = apply_routed(&mut p, &mut env, "cut 3,100 to 3,200 Hz by 12 dB", None);
    // A recipe, as one plan.
    let recipe = Recipe::builtin("spoken-word-cleanup").unwrap();
    let (_, cleaned) = apply_replay(
        &mut p,
        &mut env,
        &recipe,
        ReplayMode::Adaptive,
        Actor::user(),
        ApplyOptions::default(),
        Some("clean this recording up"),
    )
    .unwrap();
    // The model, corrected once: two exchanges, one request.
    let louder_at = p.log.len() as u64;
    let (x_louder, proposal) = ask(
        &mut p,
        &mut env,
        "make it louder",
        &[
            answer(Some("Raising the level."), &[gain(90.0)]),
            answer(Some("Raising the level."), &[gain(6.0)]),
        ],
        None,
    );
    let louder = apply_proposal(
        &mut p,
        &mut env,
        "make it louder",
        &proposal.unwrap(),
        &x_louder,
    );
    // The model asks to see an operation first.
    let brighter_at = p.log.len() as u64;
    let tilt = json!({ "params": { "db_per_octave": 1.5 }, "scope": { "kind": "clip" } });
    let (x_brighter, proposal) = ask(
        &mut p,
        &mut env,
        "make it brighter",
        &[
            answer(None, &[(DESCRIBE_TOOL, json!({ "ids": ["tilt"] }))]),
            answer(Some("A gentle tilt up."), &[("tilt", tilt)]),
        ],
        None,
    );
    let brighter = apply_proposal(
        &mut p,
        &mut env,
        "make it brighter",
        &proposal.unwrap(),
        &x_brighter,
    );
    // A spoken question, answered in words.
    let question = "what can you do about the knocking";
    let d_question = spoken(&mut p, &mut env, question);
    let (x_question, proposal) = ask(
        &mut p,
        &mut env,
        question,
        &[answer(
            Some("Which part of the recording has the knocking?"),
            &[],
        )],
        Some(&d_question),
    );
    assert!(proposal.unwrap().steps.is_empty());
    // An answer that cannot be used, even corrected.
    let (x_failed, proposal) = ask(
        &mut p,
        &mut env,
        "make it deafening",
        &[answer(None, &[gain(90.0)]), answer(None, &[gain(95.0)])],
        None,
    );
    assert!(proposal.is_none());
    // Spoken, routed, then undone: it stays listed, its step excluded.
    let d_up = spoken(&mut p, &mut env, "turn it up by 3 dB");
    let (up, up_understood) =
        apply_routed(&mut p, &mut env, "turn it up by 3 dB", Some(d_up.clone()));
    p.undo(&mut env, None).unwrap();

    let r = requests(p.log.events());
    let words: Vec<&str> = r.iter().map(|q| q.words.as_str()).collect();
    assert_eq!(
        words,
        [
            "cut 3,100 to 3,200 Hz by 12 dB",
            "clean this recording up",
            "make it louder",
            "make it brighter",
            question,
            "make it deafening",
            "turn it up by 3 dB",
        ]
    );
    let expected_cut = LoggedRequest {
        seq: r[0].seq,
        ts: r[0].ts.clone(),
        words: "cut 3,100 to 3,200 Hz by 12 dB".into(),
        via: Via::Local,
        origin: Some(Origin::Console),
        model: None,
        provider: None,
        spoken: false,
        text: Some(understood),
        problems: None,
        recipe: None,
        proposed: 0,
        step_ids: ids(&cut),
        key: cut[0].step_id.clone(),
    };
    assert_eq!(r[0], expected_cut);

    let clean = &r[1];
    assert_eq!(clean.via, Via::Local);
    assert_eq!(clean.recipe.as_deref(), Some("spoken-word-cleanup"));
    assert_eq!(
        clean.text, None,
        "a recipe's steps each have a rationale of their own"
    );
    assert_eq!(clean.step_ids, ids(&cleaned));
    assert!(clean.step_ids.len() > 1);

    let l = &r[2];
    assert_eq!(
        (l.via, l.model.as_deref(), l.provider.as_deref()),
        (Via::Model, Some("m-1"), Some("mock"))
    );
    assert_eq!(l.text.as_deref(), Some("Raising the level."));
    assert_eq!(
        (l.step_ids.clone(), l.proposed, l.problems.clone()),
        (ids(&louder), 1, None)
    );
    assert_eq!(
        l.seq, louder_at,
        "it starts at its first exchange, the one corrected"
    );
    assert_eq!(l.key, louder[0].step_id);

    let b = &r[3];
    assert_eq!(
        b.seq, brighter_at,
        "it starts where the model asked to see tilt"
    );
    assert_eq!((b.via, b.step_ids.clone()), (Via::Model, ids(&brighter)));
    assert_eq!(b.text.as_deref(), Some("A gentle tilt up."));

    let q = &r[4];
    assert_eq!(
        (q.via, q.spoken, q.step_ids.len(), q.proposed),
        (Via::Model, true, 0, 0)
    );
    assert_eq!(
        q.text.as_deref(),
        Some("Which part of the recording has the knocking?")
    );
    assert_eq!(q.problems, None);
    assert_eq!(q.key, x_question);
    assert_eq!(
        q.seq,
        seq_of(&p, &d_question),
        "it starts where it was heard"
    );
    assert_eq!(q.origin, None);

    let f = &r[5];
    assert_eq!(
        (f.via, f.step_ids.len(), f.key.as_str()),
        (Via::Model, 0, x_failed.as_str())
    );
    let problems = f.problems.as_ref().expect("why it could not be used");
    assert!(problems[0].contains("gain_db"), "{problems:?}");
    assert!(!f.spoken);

    let u = &r[6];
    assert_eq!((u.via, u.spoken), (Via::Local, true));
    assert_eq!(u.text.as_deref(), Some(up_understood.as_str()));
    assert_eq!(u.step_ids, ids(&up));
    assert_eq!(u.seq, seq_of(&p, &d_up));
    let entry = p
        .state()
        .entries
        .iter()
        .find(|e| e.step_id == up[0].step_id)
        .unwrap();
    assert!(!entry.active, "undone, it keeps its place in the stack");

    // Every event matches its schema, the spoken question's links included.
    for e in p.log.events() {
        validate(&schemas, idx["event.schema.json"], e, "event");
    }
    let asked = p
        .log
        .events()
        .iter()
        .find(|e| e["hash"] == x_question.as_str())
        .unwrap();
    assert_eq!(asked["data"]["dictation"], json!(d_question));

    // Reopened, the log gives the same requests.
    let (q, report) = Project::open(p.store.clone(), app()).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(requests(q.log.events()), r);
}

#[test]
fn a_proposal_previewed_and_decided_is_one_request_and_a_clone_inherits_steps_but_no_requests() {
    let mut env = FixedEnv::default();
    let mut p = project(&mut env);

    // Previewed, then accepted, from spoken words (the command line's way).
    let d = spoken(&mut p, &mut env, "make it louder");
    let (x_kept, proposal) = ask(
        &mut p,
        &mut env,
        "make it louder",
        &[answer(Some("Up 6 dB."), &[gain(6.0)])],
        Some(&d),
    );
    let drafts = proposal.unwrap().drafts(
        &Actor::assistant("m-1", "mock"),
        Origin::Cli,
        "make it louder",
    );
    let opts = PreviewOptions {
        exchange: Some(x_kept.clone()),
        dictation: Some(d.clone()),
        ..Default::default()
    };
    let pv = p.preview(&mut env, drafts, opts).unwrap();
    let kept = p
        .accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
        .unwrap();
    // Previewed, then rejected: its answer stays, with nothing on the stack.
    let (x_dropped, proposal) = ask(
        &mut p,
        &mut env,
        "louder still",
        &[answer(Some("Up another 3 dB."), &[gain(3.0)])],
        None,
    );
    let drafts = proposal.unwrap().drafts(
        &Actor::assistant("m-1", "mock"),
        Origin::Cli,
        "louder still",
    );
    let opts = PreviewOptions {
        exchange: Some(x_dropped.clone()),
        ..Default::default()
    };
    let pv = p.preview(&mut env, drafts, opts).unwrap();
    p.reject(&mut env, &pv.record.preview_id, None, None)
        .unwrap();

    let r = requests(p.log.events());
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].words, "make it louder");
    assert_eq!(
        (r[0].via, r[0].spoken, r[0].origin),
        (Via::Model, true, Some(Origin::Cli))
    );
    assert_eq!(r[0].step_ids, ids(&kept));
    assert_eq!(r[0].seq, seq_of(&p, &d));
    assert_eq!(r[1].words, "louder still");
    assert_eq!(
        (r[1].step_ids.len(), r[1].proposed),
        (0, 1),
        "proposed, not kept"
    );
    assert_eq!(r[1].text.as_deref(), Some("Up another 3 dB."));
    assert_eq!(r[1].key, x_dropped);

    // A clone inherits the steps; the requests were made of its parent.
    let child = p
        .clone_into(&mut env, MemStore::new(), None, None, None)
        .unwrap();
    assert!(requests(child.log.events()).is_empty());
    assert_eq!(child.state().entries.len(), kept.len());
    assert!(child.state().steps[0].inherited_from.is_some());
}

#[test]
fn an_exchange_naming_a_dictation_that_is_not_logged_is_refused() {
    let mut env = FixedEnv::default();
    let mut p = project(&mut env);
    let request = request_for(&mut p, "m-1", &[], "hello", None).unwrap();
    let before = p.log.len();
    let ex = Exchange {
        provider: "mock".into(),
        model: "m-1".into(),
        host: "example.test".into(),
        request: request.to_string(),
        response: answer(Some("Hello."), &[]),
        latency_ms: 1,
        problems: None,
        corrects: None,
        describes: None,
        dictation: Some(p.log.events()[0]["hash"].as_str().unwrap().to_string()),
    };
    assert!(record_exchange(&mut p, &mut env, &ex).is_err());
    assert_eq!(p.log.len(), before, "nothing was logged");
}
