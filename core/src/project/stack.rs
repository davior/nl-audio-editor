//! The edit stack as a projection of the event log. The log is the source of
//! truth; the stack is recomputed from it, so nothing that happened can be
//! lost, and a stack that disagrees with its log is detectable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::step::{stack_hash, Step};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PreviewRecord {
    pub preview_id: String,
    /// `step` or `plan`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    pub steps: Vec<Step>,
    pub window: [f64; 2],
    pub base_stack_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
    pub recipe: Option<Value>,
    /// The `assistant.exchange` event (its hash) the previewed steps came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<String>,
    /// The `speech.transcribed` event (its hash), when the request was spoken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictation: Option<String>,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub event_hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StackState {
    pub steps: Vec<Step>,
    /// Hash of the event that put each step on the stack.
    pub step_events: Vec<String>,
    pub stack_hash: String,
    pub previews: BTreeMap<String, PreviewRecord>,
    /// preview id → `accepted` or `rejected`.
    pub decided: BTreeMap<String, String>,
    #[cfg_attr(feature = "ts", ts(type = "Array<Record<string, unknown>>"))]
    pub ratings: Vec<Value>,
    #[cfg_attr(feature = "ts", ts(type = "Array<Record<string, unknown>>"))]
    pub annotations: Vec<Value>,
}

impl StackState {
    pub fn open_previews(&self) -> impl Iterator<Item = &PreviewRecord> {
        self.previews
            .values()
            .filter(|p| !self.decided.contains_key(&p.preview_id))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("event {seq} ({kind}): {message}")]
pub struct ProjectionError {
    pub seq: u64,
    pub kind: String,
    pub message: String,
}

fn parse<T: serde::de::DeserializeOwned>(
    v: &Value,
    seq: u64,
    kind: &str,
) -> Result<T, ProjectionError> {
    serde_json::from_value(v.clone()).map_err(|e| ProjectionError {
        seq,
        kind: kind.into(),
        message: format!("malformed data: {e}"),
    })
}

/// Fold the log into the current stack.
pub fn project(events: &[Value], source_sha256: &str) -> Result<StackState, ProjectionError> {
    let mut st = StackState {
        stack_hash: source_sha256.to_string(),
        ..Default::default()
    };
    for ev in events {
        let seq = ev["seq"].as_u64().unwrap_or(0);
        let kind = ev["type"].as_str().unwrap_or("").to_string();
        let hash = ev["hash"].as_str().unwrap_or("").to_string();
        let data = &ev["data"];
        let err = |message: String| ProjectionError {
            seq,
            kind: kind.clone(),
            message,
        };
        match kind.as_str() {
            "project.cloned_from" => {
                let steps: Vec<Step> = parse(&data["inherited_steps"], seq, &kind)?;
                st.step_events = steps
                    .iter()
                    .map(|s| {
                        s.inherited_from
                            .as_ref()
                            .map(|r| r.event_hash.clone())
                            .unwrap_or_default()
                    })
                    .collect();
                let mut h = source_sha256.to_string();
                for s in &steps {
                    h = stack_hash(&h, std::slice::from_ref(&s.render_step()));
                    if s.stack_hash.as_deref() != Some(h.as_str()) {
                        return Err(err(format!(
                            "inherited step `{}` records a stack hash that does not match",
                            s.step_id
                        )));
                    }
                }
                st.stack_hash = h;
                st.steps = steps;
            }
            "step.previewed" => {
                let mut rec: PreviewRecord = parse(data, seq, &kind)?;
                rec.seq = seq;
                rec.event_hash = hash.clone();
                st.previews.insert(rec.preview_id.clone(), rec);
            }
            "step.accepted" | "plan.accepted" => {
                let pid = data["preview_id"].as_str().unwrap_or("").to_string();
                let rec = st
                    .previews
                    .get(&pid)
                    .ok_or_else(|| err(format!("unknown preview `{pid}`")))?;
                if st.decided.contains_key(&pid) {
                    return Err(err(format!("preview `{pid}` was already decided")));
                }
                if rec.base_stack_hash != st.stack_hash {
                    return Err(err(format!(
                        "preview `{pid}` was made on a different stack"
                    )));
                }
                let steps: Vec<Step> = if kind == "step.accepted" {
                    vec![parse(&data["step"], seq, &kind)?]
                } else {
                    parse(&data["steps"], seq, &kind)?
                };
                for s in steps {
                    let expected =
                        stack_hash(&st.stack_hash, std::slice::from_ref(&s.render_step()));
                    if s.stack_hash.as_deref() != Some(expected.as_str()) {
                        return Err(err(format!(
                            "step `{}` records a stack hash that does not match its content",
                            s.step_id
                        )));
                    }
                    st.stack_hash = expected;
                    st.steps.push(s);
                    st.step_events.push(hash.clone());
                }
                st.decided.insert(pid, "accepted".into());
            }
            "step.rejected" => {
                let pid = data["preview_id"].as_str().unwrap_or("").to_string();
                if !st.previews.contains_key(&pid) {
                    return Err(err(format!("unknown preview `{pid}`")));
                }
                st.decided.insert(pid, "rejected".into());
            }
            "step.removed" => {
                let id = data["step_id"].as_str().unwrap_or("");
                match st.steps.last() {
                    Some(last) if last.step_id == id => {
                        st.steps.pop();
                        st.step_events.pop();
                        st.stack_hash = stack_hash(
                            source_sha256,
                            &st.steps.iter().map(|s| s.render_step()).collect::<Vec<_>>(),
                        );
                    }
                    _ => return Err(err(format!("`{id}` is not the top step"))),
                }
            }
            "stack.rated" => st.ratings.push(ev.clone()),
            "step.annotated" => st.annotations.push(ev.clone()),
            _ => {}
        }
    }
    Ok(st)
}
