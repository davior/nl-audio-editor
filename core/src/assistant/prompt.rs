//! What the model is shown, and nothing else: the user's words, numbers from
//! the analysis, a summary of the stack, the selection, and the registry's
//! operations as tools. No function here takes audio, so samples cannot end
//! up in a request; a check on every request backs that up.
//!
//! Tool exposure is tiered, so requests stay small as the catalogue grows:
//! the core operations are tools, every other operation is a line in the
//! instructions, and the model calls `describe_operations` to see one's
//! parameters (`build_expansion` answers it).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::route::Selection;
use crate::analysis::Features;
use crate::dataset::analysis_summary;
use crate::ops::descriptor::ToolTier;
use crate::ops::{registry, Descriptor};
use crate::project::step::Step;
use crate::scope::Scope;

/// Recorded with every exchange, so a change of wording is visible in the data.
/// Version 2: tiered tools, with the index of further operations.
/// Version 3: the model is told it may ask to see operations only once, and the
/// follow-up that answers it no longer offers `describe_operations`.
pub const PROMPT_VERSION: u32 = 3;

/// The tool the model calls to see the parameters of operations in the index.
pub const DESCRIBE_TOOL: &str = "describe_operations";

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

/// Offered operations the model is shown only in the index.
pub fn on_demand() -> Vec<&'static Descriptor> {
    offered()
        .into_iter()
        .filter(|d| d.tier == ToolTier::OnDemand)
        .collect()
}

/// The instructions: the system prompt, then one line per operation the model
/// can ask to see. Dataset records use the same text.
pub fn system_message() -> String {
    let more = on_demand();
    if more.is_empty() {
        return SYSTEM_PROMPT.to_string();
    }
    let lines: Vec<String> = more
        .iter()
        .map(|d| format!("- {}: {}. {}", d.id, d.title, d.summary))
        .collect();
    format!(
        "{SYSTEM_PROMPT}\n\nMore operations, not among the tools. To use one, first call `{DESCRIBE_TOOL}` with the ids of every operation you might need, to see their parameters; then call one of them. You can ask only once.\n{}",
        lines.join("\n")
    )
}

/// The tools: one per core operation, `plan` for several together, and
/// `describe_operations` for the rest.
pub fn tools() -> Vec<Value> {
    let ops = offered();
    let names: Vec<&str> = ops.iter().map(|d| d.id.as_str()).collect();
    let mut tools: Vec<Value> = ops
        .iter()
        .filter(|d| d.tier == ToolTier::Core)
        .map(|d| d.tool_schema())
        .collect();
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
    let more: Vec<&str> = on_demand().iter().map(|d| d.id.as_str()).collect();
    if !more.is_empty() {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": DESCRIBE_TOOL,
                "description": "See the parameters of operations listed in the instructions, before using them. It can be called once: name every operation you might need.",
                "parameters": {
                    "type": "object",
                    "required": ["ids"],
                    "additionalProperties": false,
                    "properties": {
                        "ids": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 8,
                            "items": { "type": "string", "enum": more }
                        }
                    }
                }
            }
        }));
    }
    tools
}

/// What follows the user's words in their turn. The numbers after it are
/// compact JSON, so it occurs there only as this marker.
const CONTEXT_MARK: &str = "\n\nAnalysis: ";

/// The user's turn: their words, then the numbers they are about.
pub fn user_message(words: &str, ctx: &AssistantContext) -> String {
    let compact = |v: &Value| serde_json::to_string(v).unwrap_or_default();
    format!(
        "{words}{CONTEXT_MARK}{}\nStack: {}\nSelection: {}",
        compact(&ctx.analysis),
        compact(&serde_json::to_value(&ctx.stack).unwrap_or_default()),
        ctx.selection
            .as_ref()
            .map(|s| compact(&serde_json::to_value(s).unwrap_or_default()))
            .unwrap_or_else(|| "none".into())
    )
}

/// The user's words in a request made by [`build_request`], or in a
/// follow-up to one: the turn that carries the numbers is theirs.
pub fn words_of(request: &Value) -> Option<String> {
    request["messages"]
        .as_array()?
        .iter()
        .rev()
        .filter(|m| m["role"] == "user")
        .find_map(|m| m["content"].as_str()?.rsplit_once(CONTEXT_MARK))
        .map(|(words, _)| words.to_string())
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
    let mut messages = vec![json!({ "role": "system", "content": system_message() })];
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

/// A follow-up answering a `describe_operations` call: the earlier request and
/// response are replayed, the call is answered with the operations' tool
/// schemas, and those operations become tools. The model has one chance to ask
/// (`next_round`), so `describe_operations` is withdrawn and the answer says
/// so: a tool that is offered and then refused only invites the call it
/// refuses. Any other call in the same answer is answered as not run, to be
/// made again.
pub fn build_expansion(request: &Value, response: &Value, ids: &[String]) -> Result<Value, String> {
    let reg = registry();
    let schemas = ids
        .iter()
        .map(|id| {
            reg.latest(id)
                .map(|op| op.descriptor().tool_schema())
                .map_err(|_| format!("`{id}` is not an operation in the registry"))
        })
        .collect::<Result<Vec<Value>, String>>()?;
    let mut req = request.clone();
    let message = response["choices"][0]["message"].clone();
    let calls = message["tool_calls"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut messages = req["messages"].as_array().cloned().unwrap_or_default();
    messages.push(message);
    let described = format!(
        "Described: {}. They are tools now: call one of them, `plan` or any other tool. \
         `{DESCRIBE_TOOL}` cannot be called again.\n{}",
        ids.join(", "),
        serde_json::to_string(&schemas).map_err(|e| e.to_string())?
    );
    for c in calls {
        let content = if c["function"]["name"] == DESCRIBE_TOOL {
            described.clone()
        } else {
            "Not run: the operations asked for are described alongside; call again.".to_string()
        };
        messages.push(json!({ "role": "tool", "tool_call_id": c["id"], "content": content }));
    }
    req["messages"] = Value::Array(messages);
    let mut tools = req["tools"].as_array().cloned().unwrap_or_default();
    tools.retain(|t| t["function"]["name"] != DESCRIBE_TOOL);
    for s in schemas {
        let name = &s["function"]["name"];
        if !tools.iter().any(|t| &t["function"]["name"] == name) {
            tools.push(s);
        }
    }
    req["tools"] = Value::Array(tools);
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
        assert_eq!(msgs[0]["content"], system_message());
        assert!(msgs[0]["content"]
            .as_str()
            .unwrap()
            .starts_with(SYSTEM_PROMPT));
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
        // Tiered: the core operations are tools; the rest are listed, to be described on request.
        assert!(names.contains(&DESCRIBE_TOOL));
        for id in [
            "high_pass",
            "bell",
            "gate",
            "hum_reduce",
            "loudness_normalise",
        ] {
            assert!(!names.contains(&id), "{id} is on demand, not a tool");
            assert!(
                msgs[0]["content"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("\n- {id}: ")),
                "{id} is in the index"
            );
        }
        assert!(check_no_audio(&req).is_ok());
    }

    #[test]
    fn an_expansion_answers_the_describe_call_and_adds_the_tools() {
        let req = build_request("m", &ctx(), &[], "make it brighter").unwrap();
        let args = json!({ "ids": ["tilt"] }).to_string();
        let resp = json!({ "choices": [{ "message": { "role": "assistant", "content": null,
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": DESCRIBE_TOOL, "arguments": args } }] } }] });
        let next = build_expansion(&req, &resp, &["tilt".to_string()]).unwrap();
        let msgs = next["messages"].as_array().unwrap();
        let last = msgs.last().unwrap();
        assert_eq!(last["role"], "tool");
        assert_eq!(last["tool_call_id"], "call_1");
        let answer = last["content"].as_str().unwrap();
        assert!(answer.contains("db_per_octave"));
        // The answer says what the model has now, and that it cannot look again.
        assert!(answer.starts_with("Described: tilt."), "{answer}");
        assert!(answer.contains("cannot be called again"), "{answer}");
        let names: Vec<&str> = next["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.iter().filter(|n| **n == "tilt").count(), 1);
        // The one chance to look is spent, so the tool is no longer offered; the
        // rest of the tools are as they were.
        assert!(!names.contains(&DESCRIBE_TOOL));
        assert!(names.contains(&"plan") && names.contains(&"noise_reduce"));
        assert_eq!(
            next["tools"].as_array().unwrap().len(),
            req["tools"].as_array().unwrap().len()
        );
        assert!(build_expansion(&req, &resp, &["no_such_op".to_string()]).is_err());
    }

    #[test]
    fn the_model_is_told_it_can_ask_to_see_operations_once() {
        let said = system_message();
        assert!(
            said.contains(&format!("`{DESCRIBE_TOOL}`")) && said.contains("You can ask only once.")
        );
        let tool = tools()
            .into_iter()
            .find(|t| t["function"]["name"] == DESCRIBE_TOOL)
            .unwrap();
        let description = tool["function"]["description"].as_str().unwrap();
        assert!(description.contains("once"), "{description}");
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

    #[test]
    fn the_words_are_read_back_from_a_request_and_its_follow_ups() {
        let history = [
            Turn {
                role: "user".into(),
                text: "make it louder".into(),
            },
            Turn {
                role: "assistant".into(),
                text: "Raised by 6 dB.".into(),
            },
        ];
        let req = build_request("m", &ctx(), &history, "now take the hiss out").unwrap();
        assert_eq!(words_of(&req).as_deref(), Some("now take the hiss out"));
        // A correction without tool calls adds a user turn of its own; the words stay.
        let words_only =
            json!({ "choices": [{ "message": { "role": "assistant", "content": "?" } }] });
        let fix = build_correction(&req, &words_only, &["nothing to use".into()]).unwrap();
        assert_eq!(words_of(&fix).as_deref(), Some("now take the hiss out"));
        let args = json!({ "ids": ["tilt"] }).to_string();
        let ask = json!({ "choices": [{ "message": { "role": "assistant", "content": null,
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": DESCRIBE_TOOL, "arguments": args } }] } }] });
        let more = build_expansion(&req, &ask, &["tilt".to_string()]).unwrap();
        assert_eq!(words_of(&more).as_deref(), Some("now take the hiss out"));
        assert_eq!(words_of(&json!({ "messages": [] })), None);
    }
}
