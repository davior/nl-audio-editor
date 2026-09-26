//! The edit stack as a projection of the event log. The log is the source of
//! truth; the stack is recomputed from it, so nothing that happened can be
//! lost, and a stack that disagrees with its log is detectable.
//!
//! Every step that has been on the stack keeps its place. A removed step is
//! excluded, not deleted, and can be restored to the same place. A change
//! below a step changes its input, so the change's event records every active
//! step above it again (`chain`), with its values unchanged and its stack hash
//! chained on: every stack hash can be checked from the log alone.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::step::{next_stack_hash, stack_hash, Step};

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

/// One place in the stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StackEntry {
    pub step_id: String,
    /// Plays its part in the render. An excluded step keeps its place and can
    /// be restored to it.
    pub active: bool,
    /// Edits in effect (measuring it again included; an undone edit is not counted).
    pub edits: u32,
    /// Its values were measured on audio that is no longer its input
    /// (`Step::resolved_on`).
    pub drifted: bool,
    /// Why it was removed, if it is excluded and a reason was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A change to the stack that Undo can take back and Redo repeat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StackChange {
    /// The event that made the change.
    pub event: String,
    /// `applied` (accepted steps too), `excluded`, `restored` or `edited`.
    pub kind: String,
    pub step_ids: Vec<String>,
    /// For an edit: the event in this log that holds the version before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StackState {
    /// The active steps, in order: what renders.
    pub steps: Vec<Step>,
    /// Hash of the event that put each active step's current version on the
    /// stack (for a step inherited by a clone, the event in its parent's log).
    pub step_events: Vec<String>,
    /// Every step that has been on the stack, in stack order, active or excluded.
    pub entries: Vec<StackEntry>,
    /// The current versions of the excluded steps.
    pub excluded: BTreeMap<String, Step>,
    pub stack_hash: String,
    pub previews: BTreeMap<String, PreviewRecord>,
    /// preview id → `accepted` or `rejected`.
    pub decided: BTreeMap<String, String>,
    #[cfg_attr(feature = "ts", ts(type = "Array<Record<string, unknown>>"))]
    pub ratings: Vec<Value>,
    #[cfg_attr(feature = "ts", ts(type = "Array<Record<string, unknown>>"))]
    pub annotations: Vec<Value>,
    /// Changes Undo can take back, the last one last.
    pub undo: Vec<StackChange>,
    /// Changes Undo took back that Redo can repeat, the last one last.
    pub redo: Vec<StackChange>,
}

impl StackState {
    pub fn open_previews(&self) -> impl Iterator<Item = &PreviewRecord> {
        self.previews
            .values()
            .filter(|p| !self.decided.contains_key(&p.preview_id))
    }

    /// A step on the stack, active or excluded, in its current version.
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps
            .iter()
            .find(|s| s.step_id == id)
            .or_else(|| self.excluded.get(id))
    }

    /// Where a step stands in `entries`.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.step_id == id)
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

/// The step with id `id` as an event recorded it (in `step`, `steps`, `chain`
/// or `inherited_steps`).
pub fn step_in_event(ev: &Value, id: &str) -> Option<Step> {
    let data = &ev["data"];
    let listed = ["steps", "chain", "inherited_steps"]
        .iter()
        .filter_map(|k| data[*k].as_array())
        .flatten();
    std::iter::once(&data["step"])
        .chain(listed)
        .find(|s| s["step_id"] == id)
        .and_then(|s| serde_json::from_value(s.clone()).ok())
}

/// A place in the stack while the log is folded.
struct Slot {
    step: Step,
    active: bool,
    edits: u32,
    reason: Option<String>,
    /// The event that put this version on the stack.
    event: String,
    /// The event in this log that holds this version.
    recorded_in: String,
}

/// The stack hash of the active steps below slot `k`.
fn hash_below(slots: &[Slot], k: usize, source: &str) -> String {
    slots[..k]
        .iter()
        .rev()
        .find(|s| s.active)
        .and_then(|s| s.step.stack_hash.clone())
        .unwrap_or_else(|| source.to_string())
}

/// Whether two versions of a step run the same thing with the same parameters.
fn same_values(a: &Step, b: &Step) -> bool {
    (&a.op, a.op_version, &a.params, &a.resolved, &a.scope)
        == (&b.op, b.op_version, &b.params, &b.resolved, &b.scope)
}

/// Put `chain`, the new versions of the active steps from slot `k` up, in their
/// places. Every step but `edited` must keep its values, and every stack hash
/// must chain on from the step below.
fn apply_chain(
    slots: &mut [Slot],
    k: usize,
    chain: Vec<Step>,
    edited: Option<&str>,
    event: &str,
    source: &str,
) -> Result<(), String> {
    let places: Vec<usize> = (k..slots.len()).filter(|&i| slots[i].active).collect();
    let expected: Vec<&str> = places
        .iter()
        .map(|&i| slots[i].step.step_id.as_str())
        .collect();
    let given: Vec<&str> = chain.iter().map(|s| s.step_id.as_str()).collect();
    if given != expected {
        return Err(format!(
            "the chain records steps {given:?}, but the active steps from the change up are {expected:?}"
        ));
    }
    let mut h = hash_below(slots, k, source);
    for (s, &i) in chain.into_iter().zip(&places) {
        let old = &slots[i].step;
        if Some(s.step_id.as_str()) == edited {
            if (&s.op, s.op_version, &s.scope) != (&old.op, old.op_version, &old.scope) {
                return Err(format!(
                    "an edit changes the parameters of `{}`, not its operation or scope",
                    s.step_id
                ));
            }
        } else if !same_values(&s, old) {
            return Err(format!(
                "the chain changes the values of `{}`, which the change leaves as they were",
                s.step_id
            ));
        }
        h = next_stack_hash(&h, &s.render_step());
        if s.stack_hash.as_deref() != Some(h.as_str()) {
            return Err(format!(
                "step `{}` records a stack hash that does not match its place",
                s.step_id
            ));
        }
        slots[i].step = s;
        slots[i].event = event.to_string();
        slots[i].recorded_in = event.to_string();
    }
    Ok(())
}

/// The ids an event names, unique and on the stack, with their slots.
fn named(slots: &[Slot], ids: &[String]) -> Result<Vec<usize>, String> {
    if ids.is_empty() {
        return Err("no steps named".into());
    }
    let mut out = Vec::new();
    for id in ids {
        let i = slots
            .iter()
            .position(|s| &s.step.step_id == id)
            .ok_or_else(|| format!("no step `{id}` on the stack"))?;
        if out.contains(&i) {
            return Err(format!("`{id}` is named twice"));
        }
        out.push(i);
    }
    Ok(out)
}

fn same_ids(a: &[String], b: &[String]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort();
    b.sort();
    a == b
}

/// Record a change for Undo and Redo: an undo takes back the last change, a
/// redo repeats the last change taken back, and anything else is a new change.
fn record_change(st: &mut StackState, change: StackChange, data: &Value) -> Result<(), String> {
    if let Some(h) = data["undoes"].as_str() {
        let last = st
            .undo
            .last()
            .filter(|c| c.event == h)
            .ok_or_else(|| format!("`undoes` names {h}, which is not the last change"))?;
        let inverse = match last.kind.as_str() {
            "applied" | "restored" => "excluded",
            "excluded" => "restored",
            _ => "edited",
        };
        if change.kind != inverse || !same_ids(&change.step_ids, &last.step_ids) {
            return Err(format!(
                "an undo of `{}` of {:?} must be `{inverse}` of the same steps",
                last.kind, last.step_ids
            ));
        }
        let undone = st.undo.pop().expect("checked");
        st.redo.push(undone);
    } else if let Some(h) = data["redoes"].as_str() {
        let last =
            st.redo.last().filter(|c| c.event == h).ok_or_else(|| {
                format!("`redoes` names {h}, which is not the last change undone")
            })?;
        let kind = if last.kind == "applied" {
            "restored"
        } else {
            last.kind.as_str()
        };
        if change.kind != kind || !same_ids(&change.step_ids, &last.step_ids) {
            return Err(format!(
                "a redo of `{}` of {:?} must be `{kind}` of the same steps",
                last.kind, last.step_ids
            ));
        }
        let redone = st.redo.pop().expect("checked");
        st.undo.push(redone);
    } else {
        st.undo.push(change);
        st.redo.clear();
    }
    Ok(())
}

/// Fold the log into the current stack.
pub fn project(events: &[Value], source_sha256: &str) -> Result<StackState, ProjectionError> {
    let mut st = StackState {
        stack_hash: source_sha256.to_string(),
        ..Default::default()
    };
    let mut slots: Vec<Slot> = Vec::new();
    // Event hash → position in the log, to find earlier versions of a step.
    let mut index: HashMap<String, usize> = HashMap::new();
    for (n, ev) in events.iter().enumerate() {
        let seq = ev["seq"].as_u64().unwrap_or(0);
        let kind = ev["type"].as_str().unwrap_or("").to_string();
        let hash = ev["hash"].as_str().unwrap_or("").to_string();
        let data = &ev["data"];
        let err = |message: String| ProjectionError {
            seq,
            kind: kind.clone(),
            message,
        };
        let earlier = |event: &str, id: &str| {
            index
                .get(event)
                .and_then(|&i| step_in_event(&events[i], id))
        };
        match kind.as_str() {
            "project.cloned_from" => {
                let steps: Vec<Step> = parse(&data["inherited_steps"], seq, &kind)?;
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
                slots = steps
                    .into_iter()
                    .map(|s| Slot {
                        event: s
                            .inherited_from
                            .as_ref()
                            .map(|r| r.event_hash.clone())
                            .unwrap_or_default(),
                        recorded_in: hash.clone(),
                        step: s,
                        active: true,
                        edits: 0,
                        reason: None,
                    })
                    .collect();
            }
            "step.previewed" => {
                let mut rec: PreviewRecord = parse(data, seq, &kind)?;
                rec.seq = seq;
                rec.event_hash = hash.clone();
                st.previews.insert(rec.preview_id.clone(), rec);
            }
            "step.accepted" | "plan.accepted" | "step.applied" => {
                if kind != "step.applied" {
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
                    st.decided.insert(pid, "accepted".into());
                }
                let steps: Vec<Step> = if kind == "step.accepted" {
                    vec![parse(&data["step"], seq, &kind)?]
                } else {
                    parse(&data["steps"], seq, &kind)?
                };
                if steps.is_empty() {
                    return Err(err("no steps".into()));
                }
                let ids: Vec<String> = steps.iter().map(|s| s.step_id.clone()).collect();
                for s in steps {
                    if slots.iter().any(|x| x.step.step_id == s.step_id) {
                        return Err(err(format!("step `{}` is already on the stack", s.step_id)));
                    }
                    let expected =
                        stack_hash(&st.stack_hash, std::slice::from_ref(&s.render_step()));
                    if s.stack_hash.as_deref() != Some(expected.as_str()) {
                        return Err(err(format!(
                            "step `{}` records a stack hash that does not match its content",
                            s.step_id
                        )));
                    }
                    st.stack_hash = expected;
                    slots.push(Slot {
                        step: s,
                        active: true,
                        edits: 0,
                        reason: None,
                        event: hash.clone(),
                        recorded_in: hash.clone(),
                    });
                }
                let change = StackChange {
                    event: hash.clone(),
                    kind: "applied".into(),
                    step_ids: ids,
                    before: None,
                };
                record_change(&mut st, change, data).map_err(err)?;
            }
            "step.rejected" => {
                let pid = data["preview_id"].as_str().unwrap_or("").to_string();
                if !st.previews.contains_key(&pid) {
                    return Err(err(format!("unknown preview `{pid}`")));
                }
                st.decided.insert(pid, "rejected".into());
            }
            // Written before steps could be excluded: the top step, undone.
            "step.removed" => {
                let id = data["step_id"].as_str().unwrap_or("");
                match slots.iter_mut().rev().find(|s| s.active) {
                    Some(top) if top.step.step_id == id => top.active = false,
                    _ => return Err(err(format!("`{id}` is not the top step"))),
                }
                let change = StackChange {
                    event: hash.clone(),
                    kind: "excluded".into(),
                    step_ids: vec![id.to_string()],
                    before: None,
                };
                record_change(&mut st, change, data).map_err(err)?;
            }
            "step.excluded" | "step.restored" => {
                let excluding = kind == "step.excluded";
                let ids: Vec<String> = parse(&data["step_ids"], seq, &kind)?;
                let places = named(&slots, &ids).map_err(err)?;
                for &i in &places {
                    let s = &mut slots[i];
                    if s.active != excluding {
                        return Err(err(format!(
                            "`{}` is already {}",
                            s.step.step_id,
                            if excluding { "excluded" } else { "active" }
                        )));
                    }
                    s.active = !excluding;
                    s.reason = if excluding {
                        data["reason"].as_str().map(str::to_string)
                    } else {
                        None
                    };
                }
                let k = *places.iter().min().expect("at least one");
                let chain: Vec<Step> = parse(&data["chain"], seq, &kind)?;
                apply_chain(&mut slots, k, chain, None, &hash, source_sha256).map_err(err)?;
                let change = StackChange {
                    event: hash.clone(),
                    kind: if excluding { "excluded" } else { "restored" }.into(),
                    step_ids: ids,
                    before: None,
                };
                record_change(&mut st, change, data).map_err(err)?;
            }
            "step.edited" => {
                let id = data["step_id"].as_str().unwrap_or("").to_string();
                let k = named(&slots, std::slice::from_ref(&id)).map_err(err)?[0];
                if !slots[k].active {
                    return Err(err(format!("`{id}` is excluded; restore it to edit it")));
                }
                if data["from_params"] != slots[k].step.params {
                    return Err(err(format!(
                        "`from_params` are not the parameters `{id}` had"
                    )));
                }
                let chain: Vec<Step> = parse(&data["chain"], seq, &kind)?;
                let new = chain
                    .first()
                    .ok_or_else(|| err("the chain is empty".into()))?;
                if new.params != data["to_params"] {
                    return Err(err(format!(
                        "`to_params` are not the parameters the chain records for `{id}`"
                    )));
                }
                // Undo and redo bring back a recorded version exactly.
                let target = match (data["undoes"].as_str(), data["redoes"].as_str()) {
                    // The version before the edit taken back.
                    (Some(h), _) => {
                        let before = st
                            .undo
                            .last()
                            .filter(|c| c.event == h)
                            .and_then(|c| c.before.clone());
                        Some(before.and_then(|b| earlier(&b, &id)))
                    }
                    // The version the edit repeated made.
                    (None, Some(h)) => Some(earlier(h, &id)),
                    (None, None) => None,
                };
                if let Some(t) = target {
                    let t = t.ok_or_else(|| err(format!("no earlier version of `{id}` found")))?;
                    if !same_values(new, &t) {
                        return Err(err(format!(
                            "an undo or redo of an edit must bring back the recorded version of `{id}`"
                        )));
                    }
                }
                let before = slots[k].recorded_in.clone();
                apply_chain(&mut slots, k, chain, Some(&id), &hash, source_sha256).map_err(err)?;
                // Edits in effect: an undone edit no longer counts.
                slots[k].edits = if data["undoes"].is_string() {
                    slots[k].edits.saturating_sub(1)
                } else {
                    slots[k].edits + 1
                };
                let change = StackChange {
                    event: hash.clone(),
                    kind: "edited".into(),
                    step_ids: vec![id],
                    before: Some(before),
                };
                record_change(&mut st, change, data).map_err(err)?;
            }
            "stack.rated" => st.ratings.push(ev.clone()),
            "step.annotated" => st.annotations.push(ev.clone()),
            _ => {}
        }
        st.stack_hash = hash_below(&slots, slots.len(), source_sha256);
        index.insert(hash, n);
    }
    for s in slots {
        st.entries.push(StackEntry {
            step_id: s.step.step_id.clone(),
            active: s.active,
            edits: s.edits,
            drifted: s.step.resolved_on.is_some(),
            reason: s.reason,
        });
        if s.active {
            st.steps.push(s.step);
            st.step_events.push(s.event);
        } else {
            st.excluded.insert(s.step.step_id.clone(), s.step);
        }
    }
    Ok(st)
}
