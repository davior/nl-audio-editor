//! Dataset export: the action stacks of one or more projects as training data.
//!
//! - `steps.jsonl` — one record per decided (or superseded) or applied step:
//!   state → action (exact and adaptive forms) → decision → outcome.
//!   Rejected, modified and superseded attempts are included and flagged. A
//!   step that went onto the stack also carries its fate: removed, kept, or
//!   approved (the stack was exported as it stands), with every edit, removal
//!   and restoration on the way.
//! - `episodes.jsonl` — one record per project: the final stack, ratings,
//!   lineage, and links between clones forked from the same step.
//! - `chat.jsonl` — steps with the user's words, in the OpenAI chat
//!   fine-tuning format (words and analysis → tool call).
//!
//! Records of a spoken request also carry `spoken`: what the recogniser heard
//! and whether the user changed it before sending.
//! - `manifest.json` — versions, options, counts and the SHA-256 of each file.
//!
//! Audio is left out unless asked for; records carry hashes that join back to
//! the project bundles. API keys are never stored anywhere, so never exported.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::analysis::{Features, FEATURES_VERSION};
use crate::assistant::prompt::{system_message, user_message, AssistantContext};
use crate::hash::sha256;
use crate::ops::registry;
use crate::project::stack::PreviewRecord;
use crate::project::step::Step;
use crate::project::Manifest;
use crate::provenance::AppInfo;

pub const FORMAT: &str = "nlae-dataset";
pub const FORMAT_VERSION: u32 = 1;
pub const RECORD_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioInclusion {
    #[default]
    None,
    Full,
}

#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    pub audio: AudioInclusion,
}

/// What the export needs from one project.
pub struct ProjectData {
    pub manifest: Manifest,
    pub events: Vec<Value>,
    /// Only read when audio is included.
    pub source_bytes: Option<Vec<u8>>,
    /// Features of the source (for episode records).
    pub source_features: Option<Features>,
}

/// Files of the export, by relative path.
pub struct Export {
    pub files: BTreeMap<String, Vec<u8>>,
    pub counts: BTreeMap<String, u64>,
}

/// A compact analysis summary: what the assistant would be shown. Numbers only, never audio.
pub fn analysis_summary(f: &Features) -> Value {
    json!({
        "duration_s": crate::math::round_to(f.t1 - f.t0, 3),
        "sample_rate": f.sample_rate,
        "peak_dbfs": f.peak_dbfs,
        "loudness_lufs": f.loudness_lufs,
        "noise_floor_dbfs": f.noise_floor_dbfs,
        "dc_offset": f.dc_offset,
        "clipped_samples": f.clipped_samples,
        "tonal_lines": f.tonal_lines.iter().map(|l| json!({"freq_hz": l.freq_hz, "prominence_db": l.prominence_db})).collect::<Vec<_>>(),
        "hum": f.hum,
        "quietest_region": f.quietest_region.as_ref().map(|q| json!({"t0": q.t0, "t1": q.t1, "level_dbfs": q.level_dbfs})),
        "speech_activity_ratio": f.speech_activity_ratio,
        "bandwidth_hz": f.bandwidth_hz,
    })
}

fn project_ref(m: &Manifest) -> Value {
    json!({
        "id": m.project.id,
        "name": m.project.name,
        "source_sha256": m.source.sha256,
        "lineage": m.lineage.as_ref().map(|l| json!({
            "parent_project": l.parent_project,
            "forked_at_step": l.forked_at_step,
        })),
    })
}

/// What a spoken request's record says about how it was heard.
fn spoken_ref(hash: &str, dictation: &Value) -> Value {
    json!({
        "dictation": hash,
        "heard": dictation["heard"],
        "edited": dictation["edited"],
        "provider": dictation["provider"],
        "model": dictation["model"],
    })
}

fn step_record(
    m: &Manifest,
    step: &Step,
    decision: &str,
    decision_event: Option<&Value>,
    preview: Option<&PreviewRecord>,
    previews_before: u64,
    modification: Option<&Value>,
    rating: Option<&Value>,
    prior_ops: &[String],
    spoken: Option<&Value>,
    fate: Option<&Value>,
) -> Value {
    let before = step.state_before.as_ref();
    let after = step.state_after.as_ref();
    let on_stack = matches!(decision, "accepted" | "applied");
    let mut record = json!({
        "record": "step",
        "record_version": RECORD_VERSION,
        "id": format!("{}:{}", m.project.id, step.step_id),
        "project": project_ref(m),
        "decision": {
            "outcome": decision,
            "preview_id": preview.map(|p| p.preview_id.clone()),
            "plan_id": step.plan_id,
            "at": decision_event.and_then(|e| e["ts"].as_str()),
            "by": decision_event.map(|e| e["actor"].clone()),
            "reason": decision_event.and_then(|e| e["data"]["reason"].as_str()),
            "previews_before_decision": previews_before,
            "modification": modification,
            "applied": on_stack,
        },
        "state": {
            "features": before.map(|b| &b.clip),
            "scope_features": before.and_then(|b| b.scope.as_ref()),
            "stack_before": prior_ops,
        },
        "action": {
            "op": step.op,
            "op_version": step.op_version,
            "class": step.class,
            "label": step.label,
            "params": step.params,
            "resolved": step.resolved,
            "scope": step.scope,
            "bindings": step.bindings,
            "actor": step.actor,
            "origin": step.origin,
            "intent": step.intent,
            "rationale": step.rationale,
            "note": step.note,
        },
        "outcome": {
            "measurements": step.measurements,
            "measured_over": if on_stack { json!("scope") } else { json!(preview.map(|p| p.window)) },
            "features_after": after.map(|a| &a.clip),
            "scope_features_after": after.and_then(|a| a.scope.as_ref()),
        },
        "rating": rating,
        "fate": fate,
        "hashes": {
            "source": m.source.sha256,
            "input": step.input_hash,
            "output": step.output_hash,
            "stack": step.stack_hash,
        },
    });
    if let Some(s) = spoken {
        record["action"]["spoken"] = s.clone();
    }
    record
}

/// A chat record for one step, reconstructed with the same prompt builder the
/// assistant uses: the user's words and the analysis before the step, then the
/// step as a tool call. Used where no model exchange was logged (manual work,
/// the command line, locally routed requests).
fn chat_record(
    step: &Step,
    decision: &str,
    project: &str,
    spoken: Option<&Value>,
    fate: Option<&Value>,
) -> Option<Value> {
    let intent = step.intent.as_ref()?;
    let reg = registry();
    let tool = reg
        .get(&step.op, step.op_version)
        .ok()?
        .descriptor()
        .tool_schema();
    let ctx = AssistantContext {
        analysis: step
            .state_before
            .as_ref()
            .map(|b| analysis_summary(&b.clip))
            .unwrap_or(Value::Null),
        stack: Vec::new(),
        selection: None,
    };
    let args = json!({ "params": step.params, "scope": step.scope });
    let mut record = json!({
        "messages": [
            { "role": "system", "content": system_message() },
            { "role": "user", "content": user_message(intent, &ctx) },
            { "role": "assistant", "content": step.rationale, "tool_calls": [{
                "id": format!("call_{}", step.step_id),
                "type": "function",
                "function": { "name": step.op, "arguments": serde_json::to_string(&args).unwrap_or_default() }
            }] }
        ],
        "tools": [tool],
        "metadata": { "decision": decision, "project": project, "step_id": step.step_id, "origin": step.origin, "actor": step.actor, "source": "reconstructed" },
    });
    if let Some(s) = spoken {
        record["metadata"]["spoken"] = s.clone();
    }
    if let Some(f) = fate {
        record["metadata"]["fate"] = f.clone();
    }
    Some(record)
}

/// A chat record from a logged model exchange: exactly what was sent and what
/// came back, with the decision the user made on it.
fn exchange_record(
    exchange: &Value,
    hash: &str,
    decision: &str,
    project: &str,
    preview_id: Option<&str>,
    step_ids: Vec<String>,
    spoken: Option<&Value>,
    fates: Map<String, Value>,
) -> Value {
    let mut messages = exchange["request"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    messages.push(exchange["response"]["choices"][0]["message"].clone());
    let mut record = json!({
        "messages": messages,
        "tools": exchange["request"]["tools"],
        "metadata": {
            "decision": decision,
            "project": project,
            "preview_id": preview_id,
            "step_ids": step_ids,
            "exchange": hash,
            "model": exchange["model"],
            "provider": exchange["provider"],
            "prompt_version": exchange["prompt_version"],
            "source": "exchange",
        },
    });
    if let Some(s) = spoken {
        record["metadata"]["spoken"] = s.clone();
    }
    if !fates.is_empty() {
        record["metadata"]["fates"] = Value::Object(fates);
    }
    record
}

/// The fates of steps, by id.
fn fates_of(steps: &[Step], fates: &BTreeMap<String, Value>) -> Map<String, Value> {
    steps
        .iter()
        .filter_map(|s| {
            fates
                .get(&s.step_id)
                .map(|f| (s.step_id.clone(), f.clone()))
        })
        .collect()
}

fn jsonl(records: &[Value]) -> Vec<u8> {
    let mut out = String::new();
    for r in records {
        out.push_str(&serde_json::to_string(r).expect("serialisable"));
        out.push('\n');
    }
    out.into_bytes()
}

/// What became of each step that went onto a project's stack (accepted or
/// applied), by step id: `removed` (excluded at the end), `approved` (on the
/// stack, and the stack was exported as it stands at the end) or `kept`, with
/// every edit, removal and restoration on the way, and its final values if
/// they changed.
fn fates(events: &[Value]) -> BTreeMap<String, Value> {
    struct Track {
        applied_at: Value,
        active: bool,
        exported: bool,
        edits: Vec<Value>,
        removals: Vec<Value>,
        restorations: u64,
        first: Value,
        last: Value,
    }
    let canonical = |v: &Value| crate::provenance::jcs::canonical_bytes(v).ok();
    let mut tracks: BTreeMap<String, Track> = BTreeMap::new();
    let mut exported_any = false;
    let mut changed_since_export = false;
    for ev in events {
        let data = &ev["data"];
        let at = ev["ts"].clone();
        let by = ev["actor"].clone();
        // An undo or a redo says which change it takes back or repeats.
        let linked = |mut v: Value| {
            for k in ["undoes", "redoes"] {
                if data[k].is_string() {
                    v[k] = data[k].clone();
                }
            }
            v
        };
        let ids = |key: &str| -> Vec<String> {
            serde_json::from_value(data[key].clone()).unwrap_or_default()
        };
        let kind = ev["type"].as_str().unwrap_or("");
        match kind {
            "step.accepted" | "plan.accepted" | "step.applied" => {
                let steps: Vec<Value> = if kind == "step.accepted" {
                    vec![data["step"].clone()]
                } else {
                    data["steps"].as_array().cloned().unwrap_or_default()
                };
                for s in steps {
                    let id = s["step_id"].as_str().unwrap_or("").to_string();
                    tracks.insert(
                        id,
                        Track {
                            applied_at: at.clone(),
                            active: true,
                            exported: false,
                            edits: Vec::new(),
                            removals: Vec::new(),
                            restorations: 0,
                            first: s.clone(),
                            last: s,
                        },
                    );
                }
                changed_since_export = true;
            }
            "step.removed" | "step.excluded" => {
                let removed = if kind == "step.removed" {
                    vec![data["step_id"].as_str().unwrap_or("").to_string()]
                } else {
                    ids("step_ids")
                };
                for id in removed {
                    if let Some(t) = tracks.get_mut(&id) {
                        t.active = false;
                        t.removals.push(linked(
                            json!({ "at": at, "by": by, "reason": data["reason"] }),
                        ));
                    }
                }
                changed_since_export = true;
            }
            "step.restored" => {
                for id in ids("step_ids") {
                    if let Some(t) = tracks.get_mut(&id) {
                        t.active = true;
                        t.restorations += 1;
                    }
                }
                changed_since_export = true;
            }
            "step.edited" => {
                if let Some(t) = data["step_id"].as_str().and_then(|id| tracks.get_mut(id)) {
                    t.edits.push(linked(json!({
                        "at": at,
                        "by": by,
                        "from_params": data["from_params"],
                        "to_params": data["to_params"],
                        "remeasured": data["remeasured"] == true,
                    })));
                }
                changed_since_export = true;
            }
            "render.exported" => {
                for t in tracks.values_mut().filter(|t| t.active) {
                    t.exported = true;
                }
                exported_any = true;
                changed_since_export = false;
            }
            _ => {}
        }
        // The latest version of each step the event recorded again.
        for s in data["chain"].as_array().into_iter().flatten() {
            if let Some(t) = s["step_id"].as_str().and_then(|id| tracks.get_mut(id)) {
                t.last = s.clone();
            }
        }
    }
    let exported_as_it_stands = exported_any && !changed_since_export;
    tracks
        .into_iter()
        .map(|(id, t)| {
            let outcome = if !t.active {
                "removed"
            } else if exported_as_it_stands {
                "approved"
            } else {
                "kept"
            };
            let mut fate = json!({
                "outcome": outcome,
                "exported": t.exported,
                "applied_at": t.applied_at,
                "edits": t.edits,
                "removals": t.removals,
                "restorations": t.restorations,
            });
            let changed = ["params", "resolved"]
                .iter()
                .any(|k| canonical(&t.first[*k]) != canonical(&t.last[*k]));
            if changed {
                fate["final"] =
                    json!({ "params": t.last["params"], "resolved": t.last["resolved"] });
            }
            (id, fate)
        })
        .collect()
}

/// One word for what became of a plan's steps.
fn plan_fate(steps: &[Step], fates: &BTreeMap<String, Value>) -> String {
    let outcomes: Vec<&str> = steps
        .iter()
        .filter_map(|s| fates.get(&s.step_id))
        .filter_map(|f| f["outcome"].as_str())
        .collect();
    let removed = outcomes.iter().filter(|o| **o == "removed").count();
    if outcomes.is_empty() {
        "kept".into()
    } else if removed == outcomes.len() {
        "removed".into()
    } else if removed > 0 {
        "partly_removed".into()
    } else if outcomes.iter().all(|o| *o == "approved") {
        "approved".into()
    } else {
        "kept".into()
    }
}

pub fn export(
    projects: &[ProjectData],
    opts: &ExportOptions,
    created: &str,
    app: &AppInfo,
) -> Export {
    let mut steps_out = Vec::new();
    let mut chat_out = Vec::new();
    let mut episodes = Vec::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut bump = |k: &str| *counts.entry(k.to_string()).or_insert(0) += 1;

    for pd in projects {
        let m = &pd.manifest;
        let mut previews: BTreeMap<String, PreviewRecord> = BTreeMap::new();
        let mut decided: BTreeSet<String> = BTreeSet::new();
        let mut modifications: BTreeMap<(String, String), Value> = BTreeMap::new();
        let mut ratings: Vec<Value> = Vec::new();
        // Every step that has been on the stack, in its latest version, and
        // whether it is active.
        let mut slots: Vec<(Step, bool)> = Vec::new();
        let fate = fates(&pd.events);
        let mut open_previews = 0u64;
        let mut per_project = BTreeMap::<&str, u64>::new();
        let mut exchanges: BTreeMap<String, Value> = BTreeMap::new();
        let mut dictations: BTreeMap<String, Value> = BTreeMap::new();
        for ev in &pd.events {
            if ev["type"] == "stack.rated" {
                ratings.push(json!({ "at": ev["ts"], "by": ev["actor"], "rating": ev["data"] }));
            }
        }
        let rating_for = |step: &Step| -> Option<Value> {
            ratings
                .iter()
                .rev()
                .find(|r| {
                    let t = r["rating"]["target"].as_str().unwrap_or("");
                    t == step.step_id
                        || Some(t) == step.plan_id.as_deref()
                        || Some(t) == step.stack_hash.as_deref()
                })
                .cloned()
        };
        for ev in &pd.events {
            let data = &ev["data"];
            match ev["type"].as_str().unwrap_or("") {
                "project.cloned_from" => {
                    if let Ok(s) =
                        serde_json::from_value::<Vec<Step>>(data["inherited_steps"].clone())
                    {
                        slots = s.into_iter().map(|s| (s, true)).collect();
                    }
                }
                "assistant.exchange" => {
                    exchanges.insert(ev["hash"].as_str().unwrap_or("").to_string(), data.clone());
                }
                "speech.transcribed" => {
                    dictations.insert(ev["hash"].as_str().unwrap_or("").to_string(), data.clone());
                }
                "step.previewed" => {
                    if let Ok(p) = serde_json::from_value::<PreviewRecord>(data.clone()) {
                        open_previews += 1;
                        *per_project.entry("previews").or_insert(0) += 1;
                        previews.insert(p.preview_id.clone(), p);
                    }
                }
                "step.modified" => {
                    let key = (
                        data["preview_id"].as_str().unwrap_or("").to_string(),
                        data["index"].to_string(),
                    );
                    modifications.insert(key, json!({ "from_params": data["from_params"], "to_params": data["to_params"] }));
                }
                kind @ ("step.accepted" | "plan.accepted" | "step.rejected") => {
                    let pid = data["preview_id"].as_str().unwrap_or("").to_string();
                    let Some(p) = previews.get(&pid) else {
                        continue;
                    };
                    // Earlier previews on the same stack that were never decided were
                    // superseded: tweaks tried on the way to this decision.
                    let superseded: Vec<String> = previews
                        .iter()
                        .filter(|(oid, op)| {
                            !decided.contains(*oid)
                                && **oid != pid
                                && op.seq < p.seq
                                && op.base_stack_hash == p.base_stack_hash
                        })
                        .map(|(oid, _)| oid.clone())
                        .collect();
                    let spoken_of = |p: &PreviewRecord| {
                        p.dictation
                            .as_ref()
                            .and_then(|h| dictations.get(h).map(|d| spoken_ref(h, d)))
                    };
                    for oid in &superseded {
                        let op = &previews[oid];
                        let spoken = spoken_of(op);
                        for s in &op.steps {
                            steps_out.push(step_record(
                                m,
                                s,
                                "superseded",
                                None,
                                Some(op),
                                0,
                                None,
                                None,
                                &ops_of(&slots),
                                spoken.as_ref(),
                                None,
                            ));
                            bump("superseded");
                        }
                    }
                    decided.extend(superseded);
                    decided.insert(pid.clone());
                    let before = open_previews;
                    open_previews = 0;
                    let decision = if kind == "step.rejected" {
                        "rejected"
                    } else {
                        "accepted"
                    };
                    let spoken = spoken_of(p);
                    let from_exchange = p
                        .exchange
                        .as_ref()
                        .and_then(|h| exchanges.get(h).map(|x| (h, x)));
                    let accepted: Vec<Step> = match kind {
                        "step.accepted" => serde_json::from_value(data["step"].clone())
                            .map(|s| vec![s])
                            .unwrap_or_default(),
                        "plan.accepted" => {
                            serde_json::from_value(data["steps"].clone()).unwrap_or_default()
                        }
                        _ => Vec::new(),
                    };
                    if let Some((h, x)) = from_exchange {
                        chat_out.push(exchange_record(
                            x,
                            h,
                            decision,
                            &m.project.id,
                            Some(&p.preview_id),
                            p.steps.iter().map(|s| s.step_id.clone()).collect(),
                            spoken.as_ref(),
                            fates_of(&accepted, &fate),
                        ));
                    }
                    if kind == "step.rejected" {
                        for s in &p.steps {
                            steps_out.push(step_record(
                                m,
                                s,
                                "rejected",
                                Some(ev),
                                Some(p),
                                before,
                                None,
                                None,
                                &ops_of(&slots),
                                spoken.as_ref(),
                                None,
                            ));
                            if from_exchange.is_none() {
                                if let Some(c) =
                                    chat_record(s, "rejected", &m.project.id, spoken.as_ref(), None)
                                {
                                    chat_out.push(c);
                                }
                            }
                            bump("rejected");
                        }
                        continue;
                    }
                    for (i, s) in accepted.iter().enumerate() {
                        let modification = modifications.get(&(pid.clone(), i.to_string()));
                        let f = fate.get(&s.step_id);
                        steps_out.push(step_record(
                            m,
                            s,
                            "accepted",
                            Some(ev),
                            Some(p),
                            before,
                            modification,
                            rating_for(s).as_ref(),
                            &ops_of(&slots),
                            spoken.as_ref(),
                            f,
                        ));
                        if from_exchange.is_none() {
                            if let Some(c) =
                                chat_record(s, "accepted", &m.project.id, spoken.as_ref(), f)
                            {
                                chat_out.push(c);
                            }
                        }
                        bump(if modification.is_some() {
                            "accepted_modified"
                        } else {
                            "accepted"
                        });
                        count_fate(&mut bump, f);
                        slots.push((s.clone(), true));
                    }
                    // Plan steps switched off before acceptance.
                    if let Some(off) = data["disabled"].as_array() {
                        for i in off.iter().filter_map(Value::as_u64) {
                            if let Some(s) = p.steps.get(i as usize) {
                                steps_out.push(step_record(
                                    m,
                                    s,
                                    "switched_off",
                                    Some(ev),
                                    Some(p),
                                    before,
                                    None,
                                    None,
                                    &ops_of(&slots),
                                    spoken.as_ref(),
                                    None,
                                ));
                                bump("switched_off");
                            }
                        }
                    }
                }
                "step.applied" => {
                    let applied: Vec<Step> =
                        serde_json::from_value(data["steps"].clone()).unwrap_or_default();
                    let spoken = data["dictation"]
                        .as_str()
                        .and_then(|h| dictations.get(h).map(|d| spoken_ref(h, d)));
                    let from_exchange = data["exchange"]
                        .as_str()
                        .and_then(|h| exchanges.get(h).map(|x| (h, x)));
                    if let Some((h, x)) = from_exchange {
                        chat_out.push(exchange_record(
                            x,
                            h,
                            &plan_fate(&applied, &fate),
                            &m.project.id,
                            None,
                            applied.iter().map(|s| s.step_id.clone()).collect(),
                            spoken.as_ref(),
                            fates_of(&applied, &fate),
                        ));
                    }
                    for s in &applied {
                        let f = fate.get(&s.step_id);
                        steps_out.push(step_record(
                            m,
                            s,
                            "applied",
                            Some(ev),
                            None,
                            0,
                            None,
                            rating_for(s).as_ref(),
                            &ops_of(&slots),
                            spoken.as_ref(),
                            f,
                        ));
                        if from_exchange.is_none() {
                            let outcome = f.and_then(|f| f["outcome"].as_str()).unwrap_or("kept");
                            if let Some(c) =
                                chat_record(s, outcome, &m.project.id, spoken.as_ref(), f)
                            {
                                chat_out.push(c);
                            }
                        }
                        bump("applied");
                        count_fate(&mut bump, f);
                        slots.push((s.clone(), true));
                    }
                }
                // Written before steps could be excluded: the top step, undone.
                "step.removed" => {
                    if let Some(top) = slots.iter_mut().rev().find(|(_, on)| *on) {
                        top.1 = false;
                    }
                }
                kind @ ("step.excluded" | "step.restored" | "step.edited") => {
                    let ids: Vec<String> =
                        serde_json::from_value(data["step_ids"].clone()).unwrap_or_default();
                    for (s, on) in slots.iter_mut() {
                        if ids.contains(&s.step_id) {
                            *on = kind == "step.restored";
                        }
                    }
                    // The new versions of the steps recorded again.
                    let chain: Vec<Step> =
                        serde_json::from_value(data["chain"].clone()).unwrap_or_default();
                    for c in chain {
                        if let Some(slot) = slots.iter_mut().find(|(s, _)| s.step_id == c.step_id) {
                            slot.0 = c;
                        }
                    }
                }
                _ => {}
            }
        }
        let active: Vec<&Step> = slots.iter().filter(|(_, on)| *on).map(|(s, _)| s).collect();
        let decided_count = decided.len() as u64;
        episodes.push(json!({
            "record": "episode",
            "record_version": RECORD_VERSION,
            "project": project_ref(m),
            "source_features": pd.source_features,
            "source_summary": pd.source_features.as_ref().map(analysis_summary),
            "stack": active.iter().map(|s| json!({
                "step_id": s.step_id, "op": s.op, "op_version": s.op_version, "params": s.params,
                "resolved": s.resolved, "scope": s.scope, "bindings": s.bindings, "origin": s.origin, "actor": s.actor,
            })).collect::<Vec<_>>(),
            "stack_hash": m.stack.stack_hash,
            "output_hash": active.last().and_then(|s| s.output_hash.clone()),
            "ratings": ratings,
            "counts": { "previews": per_project.get("previews").copied().unwrap_or(0), "decided_previews": decided_count, "steps": active.len(), "excluded": slots.len() - active.len() },
            "comparisons": [],
        }));
        bump("episodes");
    }

    // Link clones that fork from the same step of the same parent (and clones with their parent).
    let ids: Vec<(String, Option<String>, Option<String>)> = projects
        .iter()
        .map(|p| {
            let l = p.manifest.lineage.as_ref();
            (
                p.manifest.project.id.clone(),
                l.map(|l| l.parent_project.clone()),
                l.and_then(|l| l.forked_at_step.clone()),
            )
        })
        .collect();
    for (i, ep) in episodes.iter_mut().enumerate() {
        let (me, parent, fork) = &ids[i];
        let mut cmp = Vec::new();
        for (j, (other, oparent, ofork)) in ids.iter().enumerate() {
            if i == j {
                continue;
            }
            if parent.is_some() && parent == oparent && fork == ofork {
                cmp.push(json!({ "with": other, "relation": "sibling", "forked_at_step": fork }));
            } else if parent.as_deref() == Some(other.as_str()) {
                cmp.push(json!({ "with": other, "relation": "parent", "forked_at_step": fork }));
            } else if oparent.as_deref() == Some(me.as_str()) {
                cmp.push(json!({ "with": other, "relation": "child", "forked_at_step": ofork }));
            }
        }
        ep["comparisons"] = Value::Array(cmp);
    }

    let mut files = BTreeMap::new();
    files.insert("steps.jsonl".to_string(), jsonl(&steps_out));
    files.insert("episodes.jsonl".to_string(), jsonl(&episodes));
    files.insert("chat.jsonl".to_string(), jsonl(&chat_out));
    if opts.audio == AudioInclusion::Full {
        for pd in projects {
            if let Some(b) = &pd.source_bytes {
                let ext = pd
                    .manifest
                    .source
                    .filename
                    .rsplit_once('.')
                    .map(|(_, e)| e)
                    .unwrap_or("bin");
                files.insert(
                    format!(
                        "audio/{}.{ext}",
                        pd.manifest.source.sha256.trim_start_matches("sha256:")
                    ),
                    b.clone(),
                );
            }
        }
    }
    counts.insert("step_records".into(), steps_out.len() as u64);
    counts.insert("chat_records".into(), chat_out.len() as u64);
    let file_hashes: Map<String, Value> = files
        .iter()
        .map(|(k, v)| (k.clone(), json!(sha256(v))))
        .collect();
    let manifest = json!({
        "format": FORMAT,
        "format_version": FORMAT_VERSION,
        "record_version": RECORD_VERSION,
        "created": created,
        "app": app,
        "features_version": FEATURES_VERSION,
        "registry": registry().descriptors().map(|d| json!({"id": d.id, "version": d.version})).collect::<Vec<_>>(),
        "options": { "audio": opts.audio },
        "projects": projects.iter().map(|p| p.manifest.project.id.clone()).collect::<Vec<_>>(),
        "counts": counts,
        "files": file_hashes,
    });
    files.insert(
        "manifest.json".to_string(),
        serde_json::to_vec_pretty(&manifest).expect("serialisable"),
    );
    Export { files, counts }
}

/// The active steps' operations, in stack order.
fn ops_of(slots: &[(Step, bool)]) -> Vec<String> {
    slots
        .iter()
        .filter(|(_, on)| *on)
        .map(|(s, _)| format!("{} v{}", s.op, s.op_version))
        .collect()
}

/// Count a step's fate: `fate_approved`, `fate_kept` or `fate_removed`, and
/// `fate_edited` when its values changed after it went onto the stack.
fn count_fate(bump: &mut impl FnMut(&str), fate: Option<&Value>) {
    let Some(f) = fate else { return };
    if let Some(o) = f["outcome"].as_str() {
        bump(&format!("fate_{o}"));
    }
    if f.get("final").is_some() {
        bump("fate_edited");
    }
}
