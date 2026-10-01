//! Labels: short notes on a stretch of the recording, or on a time × frequency
//! area of it, so a place can be found again. A label changes nothing about the
//! audio or the stack, and the lanes do not draw it; the browser lists them and
//! reselects the place when one is clicked.
//!
//! A label is something the user wrote, so it is logged like every other action:
//! `label.added`, `label.edited` (its text) and `label.removed`. The list is
//! rebuilt from the log, so it is the same when the project is opened again.
//! Nothing is deleted from the log: a removed label's words are still in its
//! `label.added` and `label.edited` events. Times are seconds of the original,
//! as every position in a project is.
//!
//! (`step.annotated` is another thing: a note on a step of the stack.)

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{invalid, Project, Result};
use crate::project::store::Store;
use crate::provenance::{Actor, Env};

/// The longest a label's text may be, in characters. It is one line.
pub const MAX_TEXT_CHARS: usize = 200;

/// A label as it stands now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Label {
    pub id: String,
    /// The stretch it marks, in seconds of the original; `t0 < t1`.
    pub t0: f64,
    pub t1: f64,
    /// The frequency band (Hz) of a time × frequency area; both or neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f_lo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f_hi: Option<f64>,
    /// One line, as the user last wrote it.
    pub text: String,
    /// Where it was added in the log, and when.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub seq: u64,
    pub ts: String,
}

/// A label to add.
#[derive(Clone, Debug, PartialEq)]
pub struct NewLabel {
    pub t0: f64,
    pub t1: f64,
    pub f_lo: Option<f64>,
    pub f_hi: Option<f64>,
    pub text: String,
}

/// The text as it will be kept: without surrounding spaces, one line, not empty.
fn clean_text(text: &str) -> Result<String> {
    let t = text.trim();
    if t.is_empty() {
        return invalid("a label needs some text");
    }
    if t.chars().any(char::is_control) {
        return invalid("a label is a single line");
    }
    if t.chars().count() > MAX_TEXT_CHARS {
        return invalid(format!("a label is at most {MAX_TEXT_CHARS} characters"));
    }
    Ok(t.to_string())
}

/// The place must lie inside the recording. Out-of-range values are refused,
/// not clamped: the label would record a place the user never chose.
fn check_place(new: &NewLabel, duration_s: f64, nyquist_hz: f64) -> Result<()> {
    let (t0, t1) = (new.t0, new.t1);
    if !(t0.is_finite() && t1.is_finite() && t0 >= 0.0 && t1 > t0 && t1 <= duration_s) {
        return invalid(format!(
            "a label marks a stretch inside the recording (0 to {duration_s:.3} s), not {t0} to {t1}"
        ));
    }
    match (new.f_lo, new.f_hi) {
        (None, None) => Ok(()),
        (Some(lo), Some(hi)) => {
            if lo.is_finite() && hi.is_finite() && lo >= 0.0 && hi > lo && hi <= nyquist_hz {
                Ok(())
            } else {
                invalid(format!(
                    "a label's band lies inside 0 to {nyquist_hz} Hz, not {lo} to {hi}"
                ))
            }
        }
        _ => invalid("a label's band needs both its lower and its upper edge"),
    }
}

/// A `label.added` event as a label.
fn added(e: &Value) -> Option<Label> {
    let d = &e["data"];
    let (t0, t1) = (d["t0"].as_f64()?, d["t1"].as_f64()?);
    let band = (d["f_lo"].as_f64(), d["f_hi"].as_f64());
    Some(Label {
        id: d["label_id"].as_str()?.to_string(),
        t0,
        t1,
        f_lo: band.0.filter(|_| band.1.is_some()),
        f_hi: band.1.filter(|_| band.0.is_some()),
        text: d["text"].as_str()?.to_string(),
        seq: e["seq"].as_u64().unwrap_or(0),
        ts: e["ts"].as_str().unwrap_or_default().to_string(),
    })
}

/// The labels in a log, oldest first. Like the list of requests, it reads what
/// is there and skips what makes no sense (an edit of a label that is not
/// there): the writes are checked, and a log that was altered fails its hashes.
pub fn labels(events: &[Value]) -> Vec<Label> {
    let mut out: Vec<Label> = Vec::new();
    for e in events {
        let d = &e["data"];
        match e["type"].as_str().unwrap_or_default() {
            "label.added" => {
                if let Some(l) = added(e) {
                    if !out.iter().any(|x| x.id == l.id) {
                        out.push(l);
                    }
                }
            }
            "label.edited" => {
                if let (Some(id), Some(text)) = (d["label_id"].as_str(), d["text"].as_str()) {
                    if let Some(l) = out.iter_mut().find(|l| l.id == id) {
                        l.text = text.to_string();
                    }
                }
            }
            "label.removed" => {
                if let Some(id) = d["label_id"].as_str() {
                    out.retain(|l| l.id != id);
                }
            }
            _ => {}
        }
    }
    out
}

impl<S: Store> Project<S> {
    /// The project's labels, rebuilt from its log, oldest first.
    pub fn labels(&self) -> Vec<Label> {
        labels(self.log.events())
    }

    fn label(&self, id: &str) -> Result<Label> {
        match self.labels().into_iter().find(|l| l.id == id) {
            Some(l) => Ok(l),
            None => invalid(format!("no label `{id}`")),
        }
    }

    /// Add a label to a stretch of the recording (and, for an area, a band).
    pub fn add_label(
        &mut self,
        env: &mut dyn Env,
        new: NewLabel,
        actor: Option<Actor>,
    ) -> Result<Label> {
        self.writable()?;
        let text = clean_text(&new.text)?;
        check_place(
            &new,
            self.source.duration_s(),
            f64::from(self.source.sample_rate) / 2.0,
        )?;
        let id = env.new_id("lb");
        let mut data = json!({ "label_id": id, "t0": new.t0, "t1": new.t1, "text": text });
        if let (Some(lo), Some(hi)) = (new.f_lo, new.f_hi) {
            data["f_lo"] = json!(lo);
            data["f_hi"] = json!(hi);
        }
        let ev = self.append(env, "label.added", &actor.unwrap_or_else(Actor::user), data)?;
        Ok(added(&ev).expect("the event just logged is a label"))
    }

    /// Change a label's text. The place cannot be changed: remove it and add another.
    pub fn edit_label(
        &mut self,
        env: &mut dyn Env,
        id: &str,
        text: &str,
        actor: Option<Actor>,
    ) -> Result<Label> {
        self.writable()?;
        let before = self.label(id)?.text;
        let text = clean_text(text)?;
        self.append(
            env,
            "label.edited",
            &actor.unwrap_or_else(Actor::user),
            json!({ "label_id": id, "text": text, "before": before }),
        )?;
        self.label(id)
    }

    /// Take a label off the list. Its words stay in the log.
    pub fn remove_label(
        &mut self,
        env: &mut dyn Env,
        id: &str,
        actor: Option<Actor>,
    ) -> Result<()> {
        self.writable()?;
        self.label(id)?;
        self.append(
            env,
            "label.removed",
            &actor.unwrap_or_else(Actor::user),
            json!({ "label_id": id }),
        )?;
        Ok(())
    }
}
