//! The model's answer, checked: every proposed operation must exist, accept
//! the scope given, and have parameters within their limits (never clamped).
//! What passes becomes step drafts; what fails is explained, so the model can
//! be asked once to correct itself.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ops::registry;
use crate::project::step::{Origin, StepDraft};
use crate::provenance::Actor;
use crate::scope::Scope;

/// One operation the model proposed, validated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ProposedStep {
    pub op: String,
    pub op_version: u32,
    /// Every parameter written out, as validated.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    pub scope: Scope,
}

/// What the model answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Proposal {
    /// Empty when the model answered in words only (a question, say).
    pub steps: Vec<ProposedStep>,
    /// The model's words: its explanation, or its question.
    pub text: Option<String>,
    /// Several operations to be accepted together.
    pub plan: bool,
}

impl Proposal {
    /// Drafts for a preview, attributed to the model and carrying the user's words.
    pub fn drafts(&self, actor: &Actor, origin: Origin, words: &str) -> Vec<StepDraft> {
        self.steps
            .iter()
            .map(|s| {
                let mut d = StepDraft::new(
                    &s.op,
                    s.params.clone(),
                    s.scope.clone(),
                    actor.clone(),
                    origin,
                );
                d.op_version = Some(s.op_version);
                d.intent = Some(words.to_string());
                d.rationale = self.text.clone();
                d
            })
            .collect()
    }
}

fn check(op: &str, params: &Value, scope: &Value) -> Result<ProposedStep, String> {
    let reg = registry();
    let d = reg
        .latest(op)
        .map_err(|_| format!("`{op}` is not an operation in the registry"))?
        .descriptor();
    if d.system {
        return Err(format!(
            "`{op}` is applied automatically and cannot be proposed"
        ));
    }
    let scope: Scope = if scope.is_null() {
        Scope::Clip
    } else {
        serde_json::from_value(scope.clone()).map_err(|e| format!("{op}: scope: {e}"))?
    };
    let params = if params.is_null() {
        Value::Object(Default::default())
    } else {
        params.clone()
    };
    let params = reg
        .validate(op, d.version, &params, &scope)
        .map_err(|e| format!("{op}: {e}"))?;
    Ok(ProposedStep {
        op: op.to_string(),
        op_version: d.version,
        params,
        scope,
    })
}

/// Read a chat-completions response. `Err` lists every problem found.
pub fn parse_response(response: &Value) -> Result<Proposal, Vec<String>> {
    let message = &response["choices"][0]["message"];
    if message.is_null() {
        return Err(vec!["the response has no message".into()]);
    }
    let text = message["content"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let calls = message["tool_calls"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut steps = Vec::new();
    let mut problems = Vec::new();
    let mut plan = calls.len() > 1;
    for c in &calls {
        let name = c["function"]["name"].as_str().unwrap_or_default();
        let raw = &c["function"]["arguments"];
        let args: Value = match raw {
            Value::String(s) => match serde_json::from_str(s) {
                Ok(v) => v,
                Err(e) => {
                    problems.push(format!("{name}: the arguments are not valid JSON ({e})"));
                    continue;
                }
            },
            other => other.clone(),
        };
        if name == "plan" {
            plan = true;
            let list = args["steps"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                problems.push("plan: no steps".into());
            }
            for s in list {
                match check(
                    s["op"].as_str().unwrap_or_default(),
                    &s["params"],
                    &s["scope"],
                ) {
                    Ok(p) => steps.push(p),
                    Err(e) => problems.push(e),
                }
            }
        } else {
            match check(name, &args["params"], &args["scope"]) {
                Ok(p) => steps.push(p),
                Err(e) => problems.push(e),
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    if steps.is_empty() && text.is_none() {
        return Err(vec!["the response proposes nothing and says nothing".into()]);
    }
    Ok(Proposal { steps, text, plan })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(calls: Value, content: Value) -> Value {
        json!({ "choices": [{ "message": { "role": "assistant", "content": content, "tool_calls": calls } }] })
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args.to_string() } })
    }

    #[test]
    fn a_valid_call_becomes_a_step_with_every_parameter_written_out() {
        let r = response(
            json!([call(
                "line_reduce",
                json!({ "params": { "lines": "auto" }, "scope": { "kind": "clip" } })
            )]),
            json!("The hum at 50 Hz and its harmonics stand out; cutting them."),
        );
        let p = parse_response(&r).unwrap();
        assert!(!p.plan);
        assert_eq!(p.steps[0].op, "line_reduce");
        assert_eq!(p.steps[0].op_version, 2);
        assert_eq!(p.steps[0].params["require_in_pauses"], true);
        let d = p.drafts(
            &Actor::assistant("deepseek-chat", "deepseek"),
            Origin::Console,
            "the hum is distracting",
        );
        assert_eq!(d[0].origin, Origin::Console);
        assert_eq!(d[0].intent.as_deref(), Some("the hum is distracting"));
        assert!(d[0].rationale.as_deref().unwrap().contains("50 Hz"));
    }

    #[test]
    fn out_of_range_unknown_and_system_operations_are_refused() {
        let r = response(
            json!([
                call(
                    "gain",
                    json!({ "params": { "gain_db": 90 }, "scope": { "kind": "clip" } })
                ),
                call(
                    "teleport",
                    json!({ "params": {}, "scope": { "kind": "clip" } })
                ),
                call(
                    "limiter",
                    json!({ "params": {}, "scope": { "kind": "clip" } })
                ),
            ]),
            Value::Null,
        );
        let problems = parse_response(&r).unwrap_err();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("gain_db"));
        assert!(problems[1].contains("teleport"));
        assert!(problems[2].contains("automatically"));
    }

    #[test]
    fn plans_and_questions() {
        let r = response(
            json!([call(
                "plan",
                json!({ "steps": [
                { "op": "dc_remove", "params": {}, "scope": { "kind": "clip" } },
                { "op": "noise_reduce", "params": { "reduction_db": 15 }, "scope": { "kind": "clip" } }
            ] })
            )]),
            Value::Null,
        );
        let p = parse_response(&r).unwrap();
        assert!(p.plan);
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[1].params["reduction_db"], 15.0);

        let q =
            parse_response(&response(Value::Null, json!("Which speaker do you mean?"))).unwrap();
        assert!(q.steps.is_empty());
        assert_eq!(q.text.as_deref(), Some("Which speaker do you mean?"));
        assert!(parse_response(&response(Value::Null, Value::Null)).is_err());
    }
}
