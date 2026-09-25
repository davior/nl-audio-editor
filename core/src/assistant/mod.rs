//! The reasoning layer's core, shared by every front end: what the model is
//! shown ([`prompt`]), which requests are answered without it ([`route`]), and
//! how its answers become validated steps ([`parse`]). Sending the request is
//! left to the front end, which holds the provider's key; the key never
//! passes through here. Spoken requests are transcribed the same way
//! ([`speech`]): the core sets what the stream asks for and logs the result.

pub mod parse;
pub mod prompt;
pub mod route;
pub mod speech;

pub use parse::{parse_response, Proposal, ProposedStep};
pub use prompt::{build_correction, build_request, AssistantContext, Turn, PROMPT_VERSION};
pub use route::{route, Route, RoutedStep, Selection};
pub use speech::{listen_params, record_dictation, Dictation, Segment};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::hash::sha256;
use crate::project::store::Store;
use crate::project::{Project, ProjectError};
use crate::provenance::{Actor, Env};

/// One request to the model and its answer, as logged (`assistant.exchange`).
/// Everything that left the machine is here; the key never is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Exchange {
    pub provider: String,
    pub model: String,
    /// The host the request went to (no path, no key).
    pub host: String,
    /// The request body exactly as sent (JSON text).
    pub request: String,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub response: Value,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub latency_ms: u64,
    /// Why the answer could not be used, if it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problems: Option<Vec<String>>,
    /// The exchange this one asked the model to correct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrects: Option<String>,
}

/// The request for a project as it stands: the analysis of what the stack
/// produces now, the stack, the selection, the conversation so far.
pub fn request_for<S: Store>(
    project: &mut Project<S>,
    model: &str,
    history: &[Turn],
    words: &str,
    selection: Option<Selection>,
) -> Result<Value, ProjectError> {
    let current = project.current_render()?;
    let features = project.features_for(&current);
    let ctx = AssistantContext::new(&features, &project.state().steps, selection);
    build_request(model, &ctx, history, words).map_err(ProjectError::Invalid)
}

/// Log an exchange; returns the event's hash, which previews refer to. The
/// request is checked once more for anything that looks like sample data.
pub fn record_exchange<S: Store>(
    project: &mut Project<S>,
    env: &mut dyn Env,
    ex: &Exchange,
) -> Result<String, ProjectError> {
    let request: Value = serde_json::from_str(&ex.request)
        .map_err(|e| ProjectError::Invalid(format!("the request is not JSON: {e}")))?;
    prompt::check_no_audio(&request).map_err(ProjectError::Invalid)?;
    let mut data = json!({
        "provider": ex.provider,
        "model": ex.model,
        "host": ex.host,
        "prompt_version": PROMPT_VERSION,
        "request": request,
        "request_sha256": sha256(ex.request.as_bytes()),
        "response": ex.response,
        "latency_ms": ex.latency_ms,
    });
    if let Some(p) = &ex.problems {
        data["problems"] = json!(p);
    }
    if let Some(c) = &ex.corrects {
        data["corrects"] = json!(c);
    }
    let ev = project.record(
        env,
        "assistant.exchange",
        Some(Actor::assistant(&ex.model, &ex.provider)),
        data,
    )?;
    Ok(ev["hash"].as_str().unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::{write_wav, WavFormat};
    use crate::dataset::{self, ExportOptions, ProjectData};
    use crate::project::step::Origin;
    use crate::project::store::MemStore;
    use crate::project::{AcceptOptions, CreateOptions, PreviewOptions};
    use crate::provenance::{AppInfo, FixedEnv};

    fn project(env: &mut FixedEnv) -> Project<MemStore> {
        let mut s = crate::golden::spec_a();
        s.duration_s = 8.0;
        s.long_pause = (5.0, 6.5);
        s.whine_spans = vec![(1.0, 2.0)];
        s.bang_times = vec![3.3];
        s.clip_span = (7.0, 7.3);
        let wav = write_wav(&crate::golden::generate(&s).mix, WavFormat::F32);
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

    fn answer(gain_db: f64) -> Value {
        let args = json!({ "params": { "gain_db": gain_db }, "scope": { "kind": "clip" } });
        json!({ "choices": [{ "message": { "role": "assistant", "content": "Raising the level.",
            "tool_calls": [{ "id": "call_1", "type": "function",
                "function": { "name": "gain", "arguments": args.to_string() } }] } }] })
    }

    #[test]
    fn a_model_exchange_is_logged_linked_to_its_step_and_becomes_the_training_record() {
        let mut env = FixedEnv::default();
        let mut p = project(&mut env);
        let words = "make it louder";
        let request = request_for(&mut p, "m-1", &[], words, None).unwrap();

        // An impossible value, then the correction.
        let bad = answer(90.0);
        let problems = parse_response(&bad).unwrap_err();
        let first = record_exchange(
            &mut p,
            &mut env,
            &Exchange {
                provider: "mock".into(),
                model: "m-1".into(),
                host: "example.test".into(),
                request: request.to_string(),
                response: bad.clone(),
                latency_ms: 12,
                problems: Some(problems.clone()),
                corrects: None,
            },
        )
        .unwrap();
        let fix = build_correction(&request, &bad, &problems).unwrap();
        let good = answer(6.0);
        let second = record_exchange(
            &mut p,
            &mut env,
            &Exchange {
                provider: "mock".into(),
                model: "m-1".into(),
                host: "example.test".into(),
                request: fix.to_string(),
                response: good.clone(),
                latency_ms: 9,
                problems: None,
                corrects: Some(first.clone()),
            },
        )
        .unwrap();

        let proposal = parse_response(&good).unwrap();
        let drafts = proposal.drafts(&Actor::assistant("m-1", "mock"), Origin::Console, words);
        let opts = PreviewOptions {
            exchange: Some(second.clone()),
            ..Default::default()
        };
        let pv = p.preview(&mut env, drafts, opts).unwrap();
        assert_eq!(pv.record.exchange.as_deref(), Some(second.as_str()));
        p.accept(&mut env, &pv.record.preview_id, AcceptOptions::default())
            .unwrap();

        // The log verifies, and records what was sent and when it was corrected.
        let (q, report) = Project::open(p.store.clone(), AppInfo::new("test")).unwrap();
        assert!(report.ok(), "{:?}", report.problems);
        let exchanges: Vec<&Value> = q
            .log
            .events()
            .iter()
            .filter(|e| e["type"] == "assistant.exchange")
            .collect();
        assert_eq!(exchanges.len(), 2);
        // The log keeps JSON in canonical form (3.0 is written 3), so compare that way;
        // the hash is of the exact text that was sent.
        let canon = |v: &Value| crate::provenance::jcs::canonicalize(v).unwrap();
        assert_eq!(canon(&exchanges[0]["data"]["request"]), canon(&request));
        assert_eq!(
            exchanges[0]["data"]["request_sha256"],
            json!(sha256(request.to_string().as_bytes()))
        );
        assert_eq!(exchanges[1]["data"]["corrects"], json!(first));
        assert_eq!(exchanges[1]["actor"]["model"], "m-1");

        // The dataset's chat record is the exchange that produced the step.
        let src = p.source.clone();
        let data = [ProjectData {
            manifest: p.manifest.clone(),
            events: p.log.events().to_vec(),
            source_bytes: None,
            source_features: Some(p.features_for(&src)),
        }];
        let ex = dataset::export(
            &data,
            &ExportOptions::default(),
            "2026-09-24T00:00:00.000Z",
            &AppInfo::new("test"),
        );
        let chat: Vec<Value> = String::from_utf8(ex.files["chat.jsonl"].clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(chat.len(), 1);
        assert_eq!(chat[0]["metadata"]["source"], "exchange");
        assert_eq!(chat[0]["metadata"]["exchange"], json!(second));
        assert_eq!(canon(&chat[0]["messages"]), {
            let mut m = fix["messages"].as_array().unwrap().clone();
            m.push(good["choices"][0]["message"].clone());
            canon(&Value::Array(m))
        });
    }

    #[test]
    fn an_exchange_carrying_sample_data_is_refused() {
        let mut env = FixedEnv::default();
        let mut p = project(&mut env);
        let mut request = request_for(&mut p, "m-1", &[], "hello", None).unwrap();
        request["messages"][1]["samples"] =
            json!((0..480).map(|i| i as f64 * 1e-3).collect::<Vec<_>>());
        let before = p.log.len();
        let r = record_exchange(
            &mut p,
            &mut env,
            &Exchange {
                provider: "mock".into(),
                model: "m-1".into(),
                host: "example.test".into(),
                request: request.to_string(),
                response: json!({}),
                latency_ms: 1,
                problems: None,
                corrects: None,
            },
        );
        assert!(r.is_err());
        assert_eq!(p.log.len(), before, "nothing was logged");
    }
}
