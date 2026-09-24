//! What the model is shown, and nothing else: the user's words, numbers from
//! the analysis, a summary of the stack, the selection, and the registry's
//! operations as tools. No function here takes audio, so samples cannot end
//! up in a request; a check on every request backs that up.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::route::Selection;
use crate::analysis::Features;
use crate::dataset::analysis_summary;
use crate::ops::{registry, Descriptor};
use crate::project::step::Step;
use crate::scope::Scope;

/// Recorded with every exchange, so a change of wording is visible in the data.
pub const PROMPT_VERSION: u32 = 1;

pub const SYSTEM_PROMPT: &str = "You are the assistant in an audio editor for spoken-word, field and evidential \
recordings. The user describes what they want; you choose operations from the tools provided and set their \
parameters from the request and the analysis numbers. Never invent operations or parameters. Prefer the smallest \
change that achieves the goal: every change is previewed, and the user accepts or rejects it. When the user refers \
to the selection (\"here\", \"this part\"), use it as the scope. To propose several operations to be accepted \
together, call `plan`. If the request is unclear, ask one short question instead of calling a tool. Explain your \
choice in one or two sentences.";

/// Longest numeric array a request may contain: anything longer is data, not
/// a description, and is refused.
pub const MAX_NUMERIC_ARRAY: usize = 64;

/// A step on the stack, as the model sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepSummary {
    pub op: String,
    pub scope: Scope,
    /// The resolved values, with long lists counted rather than listed.
    pub values: Value,
}

/// Everything the model may be shown.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AssistantContext {
    /// Numbers from the analysis of what the stack currently produces.
    pub analysis: Value,
    pub stack: Vec<StepSummary>,
    pub selection: Option<Selection>,
}

impl AssistantContext {
    pub fn new(features: &Features, steps: &[Step], selection: Option<Selection>) -> Self {
        AssistantContext {
            analysis: analysis_summary(features),
            stack: steps
                .iter()
                .map(|s| StepSummary {
                    op: s.op.clone(),
                    scope: s.scope.clone(),
                    values: crate::recipe::summarise(&s.resolved),
                })
                .collect(),
            selection,
        }
    }
}

/// One earlier turn of the conversation, as words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Turn {
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
}

/// The latest version of every operation a user may apply (the final limiter
/// is applied automatically and is not offered).
pub fn offered() -> Vec<&'static Descriptor> {
    let mut latest: BTreeMap<&str, &Descriptor> = BTreeMap::new();
    for d in registry().descriptors().filter(|d| !d.system) {
        let keep = latest
            .get(d.id.as_str())
            .map_or(true, |x| d.version > x.version);
        if keep {
            latest.insert(d.id.as_str(), d);
        }
    }
    latest.into_values().collect()
}

/// The tools: one per offered operation, and `plan` for several together.
pub fn tools() -> Vec<Value> {
    let ops = offered();
    let names: Vec<&str> = ops.iter().map(|d| d.id.as_str()).collect();
    let mut tools: Vec<Value> = ops.iter().map(|d| d.tool_schema()).collect();
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "plan",
            "description": "Several operations previewed together and accepted as one unit, applied in the order given.",
            "parameters": {
                "type": "object",
                "required": ["steps"],
                "additionalProperties": false,
                "properties": {
                    "steps": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "required": ["op", "params", "scope"],
                            "properties": {
                                "op": { "type": "string", "enum": names },
                                "params": { "type": "object" },
                                "scope": { "type": "object" }
                            }
                        }
                    }
                }
            }
        }
    }));
    tools
}

/// The user's turn: their words, then the numbers they are about.
pub fn user_message(words: &str, ctx: &AssistantContext) -> String {
    let compact = |v: &Value| serde_json::to_string(v).unwrap_or_default();
    format!(
        "{words}\n\nAnalysis: {}\nStack: {}\nSelection: {}",
        compact(&ctx.analysis),
        compact(&serde_json::to_value(&ctx.stack).unwrap_or_default()),
        ctx.selection
            .as_ref()
            .map(|s| compact(&serde_json::to_value(s).unwrap_or_default()))
            .unwrap_or_else(|| "none".into())
    )
}

/// Refuse a request carrying anything that looks like sample data.
pub fn check_no_audio(v: &Value) -> Result<(), String> {
    match v {
        Value::Array(a) => {
            if a.len() > MAX_NUMERIC_ARRAY && a.iter().all(Value::is_number) {
                return Err(format!(
                    "a list of {} numbers is data, not a description; it is not sent",
                    a.len()
                ));
            }
            a.iter().try_for_each(check_no_audio)
        }
        Value::Object(m) => m.values().try_for_each(check_no_audio),
        _ => Ok(()),
    }
}

/// The chat-completions request body (OpenAI-compatible). The provider's key
/// is added by whatever sends it, never here, so it cannot be recorded.
pub fn build_request(
    model: &str,
    ctx: &AssistantContext,
    history: &[Turn],
    words: &str,
) -> Result<Value, String> {
    // The context becomes text in the user's message, so it is checked while
    // it is still structured.
    check_no_audio(&serde_json::to_value(ctx).map_err(|e| e.to_string())?)?;
    let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    let recent = history.len().saturating_sub(8);
    for t in &history[recent..] {
        let role = if t.role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        messages.push(json!({ "role": role, "content": t.text }));
    }
    messages.push(json!({ "role": "user", "content": user_message(words, ctx) }));
    let req = json!({
        "model": model,
        "messages": messages,
        "tools": tools(),
        "tool_choice": "auto",
        "temperature": 0,
    });
    check_no_audio(&req)?;
    Ok(req)
}

/// A follow-up asking the model to correct a proposal that could not be used.
/// The earlier request and response are replayed, then the problems are given
/// as the result of each tool call.
pub fn build_correction(
    request: &Value,
    response: &Value,
    problems: &[String],
) -> Result<Value, String> {
    let mut req = request.clone();
    let message = response["choices"][0]["message"].clone();
    let calls = message["tool_calls"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut messages = req["messages"].as_array().cloned().unwrap_or_default();
    messages.push(message);
    let explanation = format!(
        "This could not be used: {}. Call a tool again with valid values, or ask the user.",
        problems.join("; ")
    );
    if calls.is_empty() {
        messages.push(json!({ "role": "user", "content": explanation }));
    }
    for c in calls {
        messages.push(json!({
            "role": "tool",
            "tool_call_id": c["id"],
            "content": explanation,
        }));
    }
    req["messages"] = Value::Array(messages);
    check_no_audio(&req)?;
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioBuffer;

    fn ctx() -> AssistantContext {
        let a = AudioBuffer::mono(48000, (0..48000).map(|i| (i % 97) as f32 * 1e-4).collect());
        AssistantContext::new(&crate::analysis::features(&a, None), &[], None)
    }

    #[test]
    fn requests_carry_words_numbers_and_tools_only() {
        let req = build_request("deepseek-chat", &ctx(), &[], "the hum is distracting").unwrap();
        let msgs = req["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["content"], SYSTEM_PROMPT);
        let user = msgs[1]["content"].as_str().unwrap();
        assert!(user.starts_with("the hum is distracting\n\nAnalysis: {"));
        assert!(user.contains("Stack: []") && user.ends_with("Selection: none"));
        let names: Vec<&str> = req["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"line_reduce") && names.contains(&"plan"));
        assert!(
            !names.contains(&"limiter"),
            "the final limiter is not offered"
        );
        assert_eq!(
            names.iter().filter(|n| **n == "line_reduce").count(),
            1,
            "latest version only"
        );
        assert!(check_no_audio(&req).is_ok());
    }

    #[test]
    fn sample_data_is_refused() {
        let samples: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        let mut c = ctx();
        c.analysis["leak"] = json!(samples);
        assert!(build_request("m", &c, &[], "hello").is_err());
    }

    #[test]
    fn a_correction_answers_every_tool_call() {
        let req = build_request("m", &ctx(), &[], "cut it").unwrap();
        let resp = json!({ "choices": [{ "message": { "role": "assistant", "content": null,
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "gain", "arguments": "{}" } }] } }] });
        let fix = build_correction(&req, &resp, &["gain_db: out of range".into()]).unwrap();
        let msgs = fix["messages"].as_array().unwrap();
        let last = msgs.last().unwrap();
        assert_eq!(last["role"], "tool");
        assert_eq!(last["tool_call_id"], "call_1");
        assert!(last["content"].as_str().unwrap().contains("out of range"));
    }
}
