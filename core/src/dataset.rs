//! Dataset export: the action stacks of one or more projects as training data.
//!
//! - `steps.jsonl` — one record per decided (or superseded) step:
//!   state → action (exact and adaptive forms) → decision → outcome.
//!   Rejected, modified and superseded attempts are included and flagged.
//! - `episodes.jsonl` — one record per project: the final stack, ratings,
//!   lineage, and links between clones forked from the same step.
//! - `chat.jsonl` — steps with the user's words, in the OpenAI chat
//!   fine-tuning format (words and analysis → tool call).
//! - `manifest.json` — versions, options, counts and the SHA-256 of each file.
//!
//! Audio is left out unless asked for; records carry hashes that join back to
//! the project bundles. API keys are never stored anywhere, so never exported.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::analysis::{Features, FEATURES_VERSION};
use crate::assistant::prompt::{user_message, AssistantContext, SYSTEM_PROMPT};
use crate::hash::sha256;
use crate::ops::registry;
use crate::project::stack::PreviewRecord;
use crate::project::step::Step;
use crate::project::Manifest;
use crate::provenance::AppInfo;

pub const FORMAT: &str = "nlae-dataset";
pub const FORMAT_VERSION: u32 = 1;
pub const RECORD_VERSION: u32 = 1;

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
) -> Value {
    let before = step.state_before.as_ref();
    let after = step.state_after.as_ref();
    json!({
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
            "applied": decision == "accepted",
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
            "measured_over": if decision == "accepted" { json!("scope") } else { json!(preview.map(|p| p.window)) },
            "features_after": after.map(|a| &a.clip),
            "scope_features_after": after.and_then(|a| a.scope.as_ref()),
        },
        "rating": rating,
        "hashes": {
            "source": m.source.sha256,
            "input": step.input_hash,
            "output": step.output_hash,
            "stack": step.stack_hash,
        },
    })
}

/// A chat record for one step, reconstructed with the same prompt builder the
/// assistant uses: the user's words and the analysis before the step, then the
/// step as a tool call. Used where no model exchange was logged (manual work,
/// the command line, locally routed requests).
fn chat_record(step: &Step, decision: &str, project: &str) -> Option<Value> {
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
    Some(json!({
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_message(intent, &ctx) },
            { "role": "assistant", "content": step.rationale, "tool_calls": [{
                "id": format!("call_{}", step.step_id),
                "type": "function",
                "function": { "name": step.op, "arguments": serde_json::to_string(&args).unwrap_or_default() }
            }] }
        ],
        "tools": [tool],
        "metadata": { "decision": decision, "project": project, "step_id": step.step_id, "origin": step.origin, "actor": step.actor, "source": "reconstructed" },
    }))
}

/// A chat record from a logged model exchange: exactly what was sent and what
/// came back, with the decision the user made on it.
fn exchange_record(
    exchange: &Value,
    hash: &str,
    decision: &str,
    project: &str,
    preview: &PreviewRecord,
) -> Value {
    let mut messages = exchange["request"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    messages.push(exchange["response"]["choices"][0]["message"].clone());
    json!({
        "messages": messages,
        "tools": exchange["request"]["tools"],
        "metadata": {
            "decision": decision,
            "project": project,
            "preview_id": preview.preview_id,
            "step_ids": preview.steps.iter().map(|s| s.step_id.clone()).collect::<Vec<_>>(),
            "exchange": hash,
            "model": exchange["model"],
            "provider": exchange["provider"],
            "prompt_version": exchange["prompt_version"],
            "source": "exchange",
        },
    })
}

fn jsonl(records: &[Value]) -> Vec<u8> {
    let mut out = String::new();
    for r in records {
        out.push_str(&serde_json::to_string(r).expect("serialisable"));
        out.push('\n');
    }
    out.into_bytes()
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
        let mut stack: Vec<Step> = Vec::new();
        let mut open_previews = 0u64;
        let mut per_project = BTreeMap::<&str, u64>::new();
        let mut exchanges: BTreeMap<String, Value> = BTreeMap::new();
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
                        stack = s;
                    }
                }
                "assistant.exchange" => {
                    exchanges.insert(ev["hash"].as_str().unwrap_or("").to_string(), data.clone());
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
                    for oid in &superseded {
                        let op = &previews[oid];
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
                                &ops_of(&stack),
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
                    let from_exchange = p
                        .exchange
                        .as_ref()
                        .and_then(|h| exchanges.get(h).map(|x| (h, x)));
                    if let Some((h, x)) = from_exchange {
                        chat_out.push(exchange_record(x, h, decision, &m.project.id, p));
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
                                &ops_of(&stack),
                            ));
                            if from_exchange.is_none() {
                                if let Some(c) = chat_record(s, "rejected", &m.project.id) {
                                    chat_out.push(c);
                                }
                            }
                            bump("rejected");
                        }
                        continue;
                    }
                    let accepted: Vec<Step> = if kind == "step.accepted" {
                        serde_json::from_value(data["step"].clone())
                            .map(|s| vec![s])
                            .unwrap_or_default()
                    } else {
                        serde_json::from_value(data["steps"].clone()).unwrap_or_default()
                    };
                    for (i, s) in accepted.iter().enumerate() {
                        let modification = modifications.get(&(pid.clone(), i.to_string()));
                        steps_out.push(step_record(
                            m,
                            s,
                            "accepted",
                            Some(ev),
                            Some(p),
                            before,
                            modification,
                            rating_for(s).as_ref(),
                            &ops_of(&stack),
                        ));
                        if from_exchange.is_none() {
                            if let Some(c) = chat_record(s, "accepted", &m.project.id) {
                                chat_out.push(c);
                            }
                        }
                        bump(if modification.is_some() {
                            "accepted_modified"
                        } else {
                            "accepted"
                        });
                        stack.push(s.clone());
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
                                    &ops_of(&stack),
                                ));
                                bump("switched_off");
                            }
                        }
                    }
                }
                "step.removed" => {
                    stack.pop();
                }
                _ => {}
            }
        }
        let decided_count = decided.len() as u64;
        episodes.push(json!({
            "record": "episode",
            "record_version": RECORD_VERSION,
            "project": project_ref(m),
            "source_features": pd.source_features,
            "source_summary": pd.source_features.as_ref().map(analysis_summary),
            "stack": stack.iter().map(|s| json!({
                "step_id": s.step_id, "op": s.op, "op_version": s.op_version, "params": s.params,
                "resolved": s.resolved, "scope": s.scope, "bindings": s.bindings, "origin": s.origin, "actor": s.actor,
            })).collect::<Vec<_>>(),
            "stack_hash": m.stack.stack_hash,
            "output_hash": stack.last().and_then(|s| s.output_hash.clone()),
            "ratings": ratings,
            "counts": { "previews": per_project.get("previews").copied().unwrap_or(0), "decided_previews": decided_count, "steps": stack.len() },
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

fn ops_of(stack: &[Step]) -> Vec<String> {
    stack
        .iter()
        .map(|s| format!("{} v{}", s.op, s.op_version))
        .collect()
}
