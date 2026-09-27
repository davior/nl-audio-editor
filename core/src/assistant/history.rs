//! The requests made of a project, rebuilt from its log: what was asked, how
//! it was answered, and the steps it put on the stack. The browser shows them
//! with the stack, so a request and its answer are there again when the
//! project is reopened. Nothing is logged for this: a step's event already
//! names the exchange with the model and the dictation it came from, and an
//! exchange names the one it corrects or follows.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::parse::parse_response;
use super::prompt::words_of;
use crate::project::step::Origin;

/// How a request was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Via {
    /// Without a model: the router, a recipe, a step made by hand.
    Local,
    Model,
}

/// One request and its answer, as the log records them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LoggedRequest {
    /// Where it starts in the log: its dictation, its first exchange with the
    /// model, or the event that put its steps on the stack.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub seq: u64,
    pub ts: String,
    /// The words sent; empty for steps that came without any.
    pub words: String,
    pub via: Via,
    /// Through what its steps arrived; absent when it put none on the stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// The model asked, and its provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The words were dictated.
    pub spoken: bool,
    /// The answer in words: the model's, or what the router understood.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why the model's last answer could not be used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problems: Option<Vec<String>>,
    /// The recipe replayed, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// Operations the model's last answer proposed (none for a local request).
    pub proposed: u32,
    /// The steps it put on the stack, in stack order; empty when it put none.
    pub step_ids: Vec<String>,
    /// Names it: its first step, or else its last exchange with the model.
    pub key: String,
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v[k].as_str()
}

/// What the model said in an answer, if anything.
fn said(response: &Value) -> Option<String> {
    response["choices"][0]["message"]["content"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// What links a step event to its request: the event itself, or the preview
/// it accepted.
fn links<'a>(e: &'a Value, previews: &HashMap<&str, &'a Value>) -> Option<&'a Value> {
    match e["type"].as_str()? {
        "step.applied" => Some(&e["data"]),
        "step.accepted" | "plan.accepted" => {
            previews.get(e["data"]["preview_id"].as_str()?).copied()
        }
        _ => None,
    }
}

/// The requests in a log, oldest first. Every event that put steps on the
/// stack is one; so is the last exchange of a request to the model that put
/// none there (an answer in words, or one that could not be used). Clones'
/// inherited steps belong to none: they were asked for in the parent.
pub fn requests(events: &[Value]) -> Vec<LoggedRequest> {
    let by_hash: HashMap<&str, &Value> = events
        .iter()
        .filter_map(|e| Some((e["hash"].as_str()?, e)))
        .collect();
    let previews: HashMap<&str, &Value> = events
        .iter()
        .filter(|e| e["type"] == "step.previewed")
        .filter_map(|e| Some((e["data"]["preview_id"].as_str()?, &e["data"])))
        .collect();
    // An exchange a later one corrects or follows is not the last of its request.
    let followed: HashSet<&str> = events
        .iter()
        .filter(|e| e["type"] == "assistant.exchange")
        .flat_map(|e| {
            [
                str_of(&e["data"], "corrects"),
                str_of(&e["data"], "describes"),
            ]
        })
        .flatten()
        .collect();
    let used: HashSet<&str> = events
        .iter()
        .filter_map(|e| str_of(links(e, &previews)?, "exchange"))
        .collect();
    // The exchanges of one request to the model, last first.
    let chain = |last: &str| -> Vec<&Value> {
        let mut out = Vec::new();
        let mut at = by_hash.get(last).copied();
        while let Some(x) = at {
            if out.len() > events.len() {
                break;
            }
            out.push(x);
            let d = &x["data"];
            at = str_of(d, "corrects")
                .or_else(|| str_of(d, "describes"))
                .and_then(|h| by_hash.get(h).copied());
        }
        out
    };
    let seq_of = |e: &Value| e["seq"].as_u64().unwrap_or(0);
    let spoken_seq = |h: Option<&str>| h.and_then(|h| by_hash.get(h)).map(|d| seq_of(d));

    let mut out = Vec::new();
    for e in events {
        let d = &e["data"];
        match e["type"].as_str().unwrap_or_default() {
            kind @ ("step.applied" | "step.accepted" | "plan.accepted") => {
                let steps: Vec<&Value> = if kind == "step.accepted" {
                    vec![&d["step"]]
                } else {
                    d["steps"]
                        .as_array()
                        .map(|a| a.iter().collect())
                        .unwrap_or_default()
                };
                let Some(first) = steps.first() else { continue };
                let link = links(e, &previews).unwrap_or(&Value::Null);
                let exchanges = str_of(link, "exchange").map(chain).unwrap_or_default();
                let dictation = str_of(link, "dictation");
                let recipe = link["recipe"]["name"].as_str().map(str::to_string);
                let mut text: Vec<&str> = Vec::new();
                for s in &steps {
                    if let Some(r) = str_of(s, "rationale").filter(|r| !r.is_empty()) {
                        if !text.contains(&r) {
                            text.push(r);
                        }
                    }
                }
                let last = exchanges.first();
                let root = exchanges.last();
                let starts = [
                    Some(seq_of(e)),
                    root.map(|x| seq_of(x)),
                    spoken_seq(dictation),
                ];
                out.push(LoggedRequest {
                    seq: starts.into_iter().flatten().min().unwrap_or(0),
                    ts: str_of(e, "ts").unwrap_or_default().to_string(),
                    words: str_of(first, "intent")
                        .map(str::to_string)
                        .or_else(|| root.and_then(|x| words_of(&x["data"]["request"])))
                        .unwrap_or_default(),
                    via: if last.is_some() {
                        Via::Model
                    } else {
                        Via::Local
                    },
                    origin: serde_json::from_value(first["origin"].clone()).ok(),
                    model: last
                        .and_then(|x| str_of(&x["data"], "model"))
                        .or_else(|| str_of(&first["actor"], "model"))
                        .map(str::to_string),
                    provider: last
                        .and_then(|x| str_of(&x["data"], "provider"))
                        .or_else(|| str_of(&first["actor"], "provider"))
                        .map(str::to_string),
                    spoken: dictation.is_some(),
                    text: if recipe.is_some() || text.is_empty() {
                        None
                    } else {
                        Some(text.join(" "))
                    },
                    problems: None,
                    recipe,
                    proposed: last
                        .and_then(|x| parse_response(&x["data"]["response"]).ok())
                        .map_or(0, |p| p.steps.len() as u32),
                    step_ids: steps
                        .iter()
                        .filter_map(|s| str_of(s, "step_id").map(str::to_string))
                        .collect(),
                    key: str_of(first, "step_id").unwrap_or_default().to_string(),
                });
            }
            "assistant.exchange" => {
                let Some(h) = e["hash"].as_str() else {
                    continue;
                };
                if followed.contains(h) || used.contains(h) {
                    continue;
                }
                let exchanges = chain(h);
                let root = exchanges.last().copied().unwrap_or(e);
                let dictation = exchanges
                    .iter()
                    .find_map(|x| str_of(&x["data"], "dictation"));
                let response = &d["response"];
                let starts = [Some(seq_of(root)), spoken_seq(dictation)];
                out.push(LoggedRequest {
                    seq: starts.into_iter().flatten().min().unwrap_or(0),
                    ts: str_of(root, "ts").unwrap_or_default().to_string(),
                    words: words_of(&root["data"]["request"]).unwrap_or_default(),
                    via: Via::Model,
                    origin: None,
                    model: str_of(d, "model").map(str::to_string),
                    provider: str_of(d, "provider").map(str::to_string),
                    spoken: dictation.is_some(),
                    text: said(response),
                    problems: serde_json::from_value(d["problems"].clone()).ok(),
                    recipe: None,
                    proposed: parse_response(response).map_or(0, |p| p.steps.len() as u32),
                    step_ids: Vec::new(),
                    key: h.to_string(),
                });
            }
            _ => {}
        }
    }
    out.sort_by_key(|r| r.seq);
    out
}
