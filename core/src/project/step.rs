//! The step object: one recorded action, in the same shape whatever produced
//! it (console, panel, command line, recipe, automation). Only `actor` and
//! `origin` differ between paths.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::analysis::Features;
use crate::engine::RenderStep;
use crate::hash::sha256_parts;
use crate::ops::OpClass;
use crate::provenance::jcs;
use crate::provenance::Actor;
use crate::scope::Scope;

/// Through what a step arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Origin {
    Console,
    Panel,
    Cli,
    Recipe,
    Automation,
    Api,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum BindingSource {
    /// Stated by the assistant or a recipe.
    Declared,
    /// Worked out from the clip's analysis when the step was accepted.
    Inferred,
}

/// How a value relates to the clip's own analysis — the adaptive form. On
/// another clip the value is re-derived from that clip's analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Binding {
    /// `tonal_lines`, `peak_dbfs`, `quietest_region`, `clip`.
    pub feature: String,
    /// Which item of a list feature, e.g. `{"near_hz": 3150, "rank": 2}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
    pub select: Option<Value>,
    /// `offset` (value = recorded − feature), `target` (value is the target the
    /// feature is brought to), `equals`, `fraction` (of the clip's duration).
    pub relation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    pub source: BindingSource,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Features of a step's input or output: the whole clip, and the step's time
/// scope when it has one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StateSnapshot {
    pub clip: Features,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Features>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct InheritedRef {
    pub project: String,
    pub step_id: String,
    pub event_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Step {
    pub step_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    pub op: String,
    pub op_version: u32,
    /// Parameters as given, every one written out; may contain `auto`.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    /// The concrete values used on this clip — the exact form.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub resolved: Value,
    pub scope: Scope,
    /// The adaptive form, keyed by parameter (`f_lo`, `scope.t0`, …).
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    pub actor: Actor,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub class: OpClass,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_before: Option<StateSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_after: Option<StateSnapshot>,
    /// What the DSP measured: over the preview window in a preview, over the
    /// whole clip once accepted.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub measurements: Map<String, Value>,
    pub input_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_hash: Option<String>,
    /// Render hash of the input `resolved` was measured on, when that is no
    /// longer the step's input: a step below it was removed, restored or edited
    /// since, and this step kept its values. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<InheritedRef>,
}

impl Step {
    /// What rendering needs.
    pub fn render_step(&self) -> RenderStep {
        RenderStep {
            op: self.op.clone(),
            op_version: self.op_version,
            resolved: self.resolved.clone(),
            scope: self.scope.clone(),
        }
    }

    /// The canonical core the stack hash covers: what was run, exactly.
    pub fn core(&self) -> Value {
        json!({ "op": self.op, "op_version": self.op_version, "resolved": self.resolved, "scope": self.scope })
    }
}

/// `stack_hash_n = H(stack_hash_{n−1}, canonical step core)`; `stack_hash_0` is
/// the source's SHA-256. Equal stack hashes mean equal renders.
pub fn next_stack_hash(prev: &str, step: &RenderStep) -> String {
    let core = json!({ "op": step.op, "op_version": step.op_version, "resolved": step.resolved, "scope": step.scope });
    let bytes = jcs::canonical_bytes(&core).expect("resolved values are finite");
    sha256_parts(&[prev.as_bytes(), &bytes])
}

pub fn stack_hash(source_sha256: &str, steps: &[RenderStep]) -> String {
    steps
        .iter()
        .fold(source_sha256.to_string(), |h, s| next_stack_hash(&h, s))
}

/// A step as proposed, before it is resolved on the clip.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StepDraft {
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_version: Option<u32>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    #[serde(default)]
    pub scope: Scope,
    pub actor: Actor,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Declared bindings (the assistant's or a recipe's).
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    /// Exact replay: use these resolved values instead of resolving on this clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
    pub resolved: Option<Value>,
}

impl StepDraft {
    pub fn new(op: &str, params: Value, scope: Scope, actor: Actor, origin: Origin) -> Self {
        StepDraft {
            op: op.to_string(),
            op_version: None,
            params,
            scope,
            actor,
            origin,
            intent: None,
            rationale: None,
            note: None,
            bindings: BTreeMap::new(),
            resolved: None,
        }
    }
}
