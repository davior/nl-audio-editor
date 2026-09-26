//! The append-only, hash-chained event log (JSON Lines).
//!
//! Each line is one event in canonical JSON:
//! `{v, seq, ts, type, actor, app, data, prev, hash}`.
//! `hash` is the SHA-256 of the canonical form of the event without `hash`;
//! `prev` is the previous event's `hash`. Altering, deleting or reordering any
//! line breaks the chain at that line, and verification reports the first
//! failure. The log is the source of truth; the edit stack is a projection of it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::env::Env;
use super::jcs;
use crate::hash::{sha256, ZERO_HASH};

pub const EVENT_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ActorKind {
    /// A person, through any interface.
    User,
    /// The AI assistant proposing or taking an action.
    Assistant,
    /// Automated processing (batch runs, replays without review).
    Automation,
    /// The application itself (imports, analysis, housekeeping records).
    System,
}

/// Who performed an action. For the assistant, the model and provider are
/// recorded; API keys never are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Actor {
    pub kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl Actor {
    pub fn user() -> Self {
        Actor {
            kind: ActorKind::User,
            name: None,
            model: None,
            provider: None,
        }
    }
    pub fn system() -> Self {
        Actor {
            kind: ActorKind::System,
            name: None,
            model: None,
            provider: None,
        }
    }
    pub fn assistant(model: &str, provider: &str) -> Self {
        Actor {
            kind: ActorKind::Assistant,
            name: None,
            model: Some(model.into()),
            provider: Some(provider.into()),
        }
    }
    pub fn automation(name: &str) -> Self {
        Actor {
            kind: ActorKind::Automation,
            name: Some(name.into()),
            model: None,
            provider: None,
        }
    }
}

/// Which program wrote an event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// `cli`, `web`, `desktop`, `test`.
    pub build: String,
}

impl AppInfo {
    pub fn new(build: &str) -> Self {
        AppInfo {
            name: "nlae".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            build: build.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct VerifyFailure {
    /// 1-based line number in the log file.
    pub line: u64,
    /// The `seq` the line should have had.
    pub expected_seq: u64,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Verification {
    pub ok: bool,
    /// Number of events that verified before the first failure (or all of them).
    pub verified: u64,
    pub first_failure: Option<VerifyFailure>,
    /// Hash of the last verified event.
    pub head: String,
}

#[derive(Clone, Debug)]
pub struct EventLog {
    genesis_prev: String,
    events: Vec<Value>,
}

impl EventLog {
    /// An empty log. `genesis_prev` is the all-zero hash for an original project,
    /// or the parent's `project.clone_made` event hash for a clone.
    pub fn new(genesis_prev: &str) -> Self {
        EventLog {
            genesis_prev: genesis_prev.to_string(),
            events: Vec::new(),
        }
    }

    pub fn new_original() -> Self {
        Self::new(ZERO_HASH)
    }

    pub fn genesis_prev(&self) -> &str {
        &self.genesis_prev
    }

    pub fn events(&self) -> &[Value] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn head_hash(&self) -> String {
        self.events
            .last()
            .and_then(|e| e["hash"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| self.genesis_prev.clone())
    }

    /// Append an event and return it (with `seq`, `prev` and `hash` filled in).
    pub fn append(
        &mut self,
        env: &mut dyn Env,
        app: &AppInfo,
        kind: &str,
        actor: &Actor,
        data: Value,
    ) -> Value {
        let mut ev = json!({
            "v": EVENT_VERSION,
            "seq": self.events.len() as u64,
            "ts": env.now(),
            "type": kind,
            "actor": actor,
            "app": app,
            "data": data,
            "prev": self.head_hash(),
        });
        let hash = event_hash(&ev).expect("events are built from finite values");
        ev.as_object_mut()
            .expect("object")
            .insert("hash".into(), Value::String(hash));
        // Keep exactly what is written: numbers as the canonical form reads
        // back (`-36`, not `-36.0`), so a reopened log equals the live one.
        let text = jcs::canonicalize(&ev).expect("events are canonicalisable");
        let ev: Value = serde_json::from_str(&text).expect("canonical JSON parses");
        self.events.push(ev.clone());
        ev
    }

    /// Take back the event just appended, before it is written anywhere.
    pub(crate) fn pop(&mut self) -> Option<Value> {
        self.events.pop()
    }

    /// One canonical JSON line (with trailing newline) for an event.
    pub fn line_of(event: &Value) -> String {
        let mut s = jcs::canonicalize(event).expect("events are canonicalisable");
        s.push('\n');
        s
    }

    pub fn to_jsonl(&self) -> String {
        self.events.iter().map(Self::line_of).collect()
    }

    /// Parse and verify. On failure the log holds only the events that verified,
    /// and the report says where and why it broke.
    pub fn parse(text: &str, expected_genesis: Option<&str>) -> (Self, Verification) {
        let mut events = Vec::new();
        let mut first_failure = None;
        let mut genesis = expected_genesis.map(str::to_string);
        let mut prev = genesis.clone();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let expected_seq = events.len() as u64;
            let fail = |reason: String| VerifyFailure {
                line: i as u64 + 1,
                expected_seq,
                reason,
            };
            let ev: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    first_failure = Some(fail(format!("line is not valid JSON: {e}")));
                    break;
                }
            };
            if ev["seq"].as_u64() != Some(expected_seq) {
                first_failure = Some(fail(format!(
                    "sequence break: expected seq {expected_seq}, found {} (a line was removed, inserted or reordered)",
                    ev["seq"]
                )));
                break;
            }
            let ev_prev = ev["prev"].as_str().unwrap_or("").to_string();
            match &prev {
                Some(p) if *p != ev_prev => {
                    first_failure = Some(fail(format!(
                        "broken link: prev is {ev_prev}, expected {p}"
                    )));
                    break;
                }
                None => {
                    genesis = Some(ev_prev.clone());
                }
                _ => {}
            }
            let stated = ev["hash"].as_str().unwrap_or("").to_string();
            match event_hash(&ev) {
                Ok(h) if h == stated => {}
                Ok(h) => {
                    first_failure = Some(fail(format!(
                        "hash mismatch: the event was altered (stated {stated}, computed {h})"
                    )));
                    break;
                }
                Err(e) => {
                    first_failure = Some(fail(format!("cannot canonicalise event: {e}")));
                    break;
                }
            }
            prev = Some(stated);
            events.push(ev);
        }
        let log = EventLog {
            genesis_prev: genesis.unwrap_or_else(|| ZERO_HASH.to_string()),
            events,
        };
        let verification = Verification {
            ok: first_failure.is_none(),
            verified: log.events.len() as u64,
            first_failure,
            head: log.head_hash(),
        };
        (log, verification)
    }
}

/// SHA-256 of the canonical form of `event` without its `hash` field.
pub fn event_hash(event: &Value) -> Result<String, jcs::JcsError> {
    let mut obj: Map<String, Value> = event.as_object().cloned().unwrap_or_default();
    obj.remove("hash");
    Ok(sha256(&jcs::canonical_bytes(&Value::Object(obj))?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::FixedEnv;

    fn sample_log() -> String {
        let mut env = FixedEnv::default();
        let app = AppInfo::new("test");
        let mut log = EventLog::new_original();
        for i in 0..5 {
            log.append(
                &mut env,
                &app,
                "test.event",
                &Actor::user(),
                json!({"i": i, "note": "x"}),
            );
        }
        log.to_jsonl()
    }

    #[test]
    fn intact_log_verifies() {
        let text = sample_log();
        let (log, v) = EventLog::parse(&text, Some(ZERO_HASH));
        assert!(v.ok, "{v:?}");
        assert_eq!(v.verified, 5);
        assert_eq!(log.to_jsonl(), text, "re-serialisation is byte-identical");
    }

    #[test]
    fn edited_byte_is_reported_at_its_line() {
        let text = sample_log().replacen("\"i\":2", "\"i\":7", 1);
        let (_, v) = EventLog::parse(&text, Some(ZERO_HASH));
        let f = v.first_failure.unwrap();
        assert_eq!((f.line, f.expected_seq), (3, 2));
        assert!(f.reason.starts_with("hash mismatch"), "{}", f.reason);
        assert_eq!(v.verified, 2);
    }

    #[test]
    fn deleted_line_is_reported() {
        let full = sample_log();
        let lines: Vec<&str> = full.lines().collect();
        let text: String = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, l)| format!("{l}\n"))
            .collect();
        let (_, v) = EventLog::parse(&text, Some(ZERO_HASH));
        let f = v.first_failure.unwrap();
        assert_eq!((f.line, f.expected_seq), (2, 1));
        assert!(f.reason.starts_with("sequence break"), "{}", f.reason);
    }

    #[test]
    fn reordered_lines_are_reported() {
        let full = sample_log();
        let mut lines: Vec<&str> = full.lines().collect();
        lines.swap(3, 4);
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let (_, v) = EventLog::parse(&text, Some(ZERO_HASH));
        let f = v.first_failure.unwrap();
        assert_eq!((f.line, f.expected_seq), (4, 3));
    }

    #[test]
    fn rehashed_edit_breaks_the_next_link() {
        // An attacker who edits an event and recomputes its hash still breaks the chain.
        let text = sample_log();
        let mut events: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        events[1]["data"]["i"] = json!(99);
        let h = event_hash(&events[1]).unwrap();
        events[1]["hash"] = json!(h);
        let text: String = events.iter().map(EventLog::line_of).collect();
        let (_, v) = EventLog::parse(&text, Some(ZERO_HASH));
        let f = v.first_failure.unwrap();
        assert_eq!(f.expected_seq, 2);
        assert!(f.reason.starts_with("broken link"), "{}", f.reason);
    }

    #[test]
    fn wrong_genesis_is_reported() {
        let text = sample_log();
        let (_, v) = EventLog::parse(&text, Some("sha256:ff"));
        assert_eq!(v.first_failure.unwrap().expected_seq, 0);
    }
}
