//! Recipes: a stack (or part of one) saved to replay on other clips.
//!
//! Replay is `exact` (the recorded resolved values) or `adaptive`, the default:
//! bound values are re-derived from the new clip's analysis and `auto` values
//! are re-measured. Every replay starts with a dry-run diff listing each value
//! that will be used, what was recorded, and why it changed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::analysis::FEATURES_VERSION;
use crate::hash::sha256;
use crate::ops::registry;
use crate::project::bindings::{self, DiffEntry};
use crate::project::step::{Binding, Origin, Step, StepDraft};
use crate::project::store::Store;
use crate::project::{Project, ProjectError};
use crate::provenance::{jcs, Actor};
use crate::scope::Scope;

pub const FORMAT: &str = "nlae-recipe";
pub const FORMAT_VERSION: u32 = 1;

/// The current built-in clean-up (version 2: lines must also stand out in the
/// speech pauses).
pub const BUILTIN_SPOKEN_WORD_CLEANUP: &str =
    include_str!("../../recipes/builtin/spoken-word-cleanup.v2.json");
/// Version 1, kept so replays recorded with it can be repeated exactly.
pub const BUILTIN_SPOKEN_WORD_CLEANUP_V1: &str =
    include_str!("../../recipes/builtin/spoken-word-cleanup.v1.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RecipeOrigin {
    pub project: String,
    pub source_sha256: String,
    pub stack_hash: String,
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RecipeStep {
    pub op: String,
    pub op_version: u32,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    /// The exact form as recorded (absent in built-in recipes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
    pub resolved: Option<Value>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Recipe {
    pub format: String,
    pub format_version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_from: Option<RecipeOrigin>,
    pub features_version: u32,
    pub steps: Vec<RecipeStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ReplayMode {
    Exact,
    Adaptive,
}

impl Recipe {
    pub fn parse(text: &str) -> Result<Recipe, ProjectError> {
        let r: Recipe = serde_json::from_str(text)
            .map_err(|e| ProjectError::Json("recipe".into(), e.to_string()))?;
        if r.format != FORMAT || r.format_version != FORMAT_VERSION {
            return Err(ProjectError::Invalid(format!(
                "not a recipe this version can read ({} v{})",
                r.format, r.format_version
            )));
        }
        let reg = registry();
        for (i, s) in r.steps.iter().enumerate() {
            reg.validate(&s.op, s.op_version, &s.params, &s.scope)
                .map_err(|e| ProjectError::Invalid(format!("recipe step {i} ({}): {e}", s.op)))?;
        }
        Ok(r)
    }

    pub fn builtin(name: &str) -> Option<Recipe> {
        match name {
            "spoken-word-cleanup" | "spoken-word-cleanup@2" => Some(
                Recipe::parse(BUILTIN_SPOKEN_WORD_CLEANUP).expect("built-in recipes are valid"),
            ),
            "spoken-word-cleanup@1" => Some(
                Recipe::parse(BUILTIN_SPOKEN_WORD_CLEANUP_V1).expect("built-in recipes are valid"),
            ),
            _ => None,
        }
    }

    /// SHA-256 of the canonical JSON.
    pub fn hash(&self) -> String {
        sha256(
            &jcs::canonical_bytes(&serde_json::to_value(self).expect("serialisable"))
                .expect("finite"),
        )
    }

    /// Save accepted steps `from..=to` (indices into the stack) as a recipe.
    pub fn from_steps(name: &str, steps: &[Step], origin: Option<RecipeOrigin>) -> Recipe {
        Recipe {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            name: name.into(),
            title: None,
            description: String::new(),
            created_from: origin,
            features_version: FEATURES_VERSION,
            steps: steps
                .iter()
                .map(|s| RecipeStep {
                    op: s.op.clone(),
                    op_version: s.op_version,
                    params: s.params.clone(),
                    resolved: Some(s.resolved.clone()),
                    scope: s.scope.clone(),
                    bindings: s.bindings.clone(),
                    optional: false,
                    enabled: true,
                    title: None,
                    intent: s.intent.clone(),
                    rationale: s.rationale.clone(),
                })
                .collect(),
        }
    }

    /// Save part of a project's stack.
    pub fn from_project<S: Store>(
        project: &Project<S>,
        name: &str,
        from: usize,
        to: usize,
    ) -> Result<Recipe, ProjectError> {
        let st = project.state();
        if st.steps.is_empty() || from > to || to >= st.steps.len() {
            return Err(ProjectError::Invalid(format!(
                "steps {from}–{to} are not on the stack ({} steps)",
                st.steps.len()
            )));
        }
        // Time edits belong to one recording: they are not carried to another.
        let steps: Vec<Step> = st.steps[from..=to]
            .iter()
            .filter(|s| !crate::timeline::is_edit(&s.op))
            .cloned()
            .collect();
        if steps.is_empty() {
            return Err(ProjectError::Invalid(
                "only time edits in that range: they belong to this recording and are not saved in recipes".into(),
            ));
        }
        let steps = &steps[..];
        let origin = RecipeOrigin {
            project: project.id().to_string(),
            source_sha256: project.source_sha256().to_string(),
            stack_hash: steps
                .last()
                .and_then(|s| s.stack_hash.clone())
                .unwrap_or_default(),
            steps: steps.iter().map(|s| s.step_id.clone()).collect(),
        };
        Ok(Recipe::from_steps(name, steps, Some(origin)))
    }
}

/// Drafts ready to preview as a plan, and the dry-run diff.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReplayPlan {
    pub recipe: String,
    pub recipe_hash: String,
    pub mode: ReplayMode,
    pub drafts: Vec<StepDraft>,
    /// Recipe step index for each draft.
    pub step_indices: Vec<usize>,
    pub diff: Vec<DiffEntry>,
    /// Recipe steps not included (switched off or optional).
    pub skipped: Vec<usize>,
}

/// Replay a recipe as one plan, as an interface does: record the dry-run diff
/// (`recipe.replayed`), then preview the plan. `intent` is the user's words;
/// `opts` gives the window and what the request came from (its plan and
/// recipe fields are set here).
pub fn preview_replay<S: Store>(
    project: &mut Project<S>,
    env: &mut dyn crate::provenance::Env,
    recipe: &Recipe,
    mode: ReplayMode,
    actor: Actor,
    opts: crate::project::PreviewOptions,
    intent: Option<&str>,
) -> Result<(ReplayPlan, crate::project::Preview), ProjectError> {
    let mut plan = plan_replay(project, recipe, mode, false, actor)?;
    if let Some(w) = intent {
        for d in plan.drafts.iter_mut() {
            d.intent = Some(w.to_string());
        }
    }
    project.record(
        env,
        "recipe.replayed",
        None,
        json!({ "recipe": plan.recipe, "hash": plan.recipe_hash, "mode": plan.mode, "dry_run": false, "diff": plan.diff }),
    )?;
    let opts = crate::project::PreviewOptions {
        plan: true,
        recipe: Some(json!({ "name": plan.recipe, "hash": plan.recipe_hash, "mode": plan.mode })),
        ..opts
    };
    let pv = project.preview(env, plan.drafts.clone(), opts)?;
    Ok((plan, pv))
}

/// Replay a recipe as one plan and apply it at once: record the dry-run diff
/// (`recipe.replayed`), then apply the plan (`step.applied`). `intent` is the
/// user's words; `opts` gives what the request came from (its plan and recipe
/// fields are set here).
pub fn apply_replay<S: Store>(
    project: &mut Project<S>,
    env: &mut dyn crate::provenance::Env,
    recipe: &Recipe,
    mode: ReplayMode,
    actor: Actor,
    opts: crate::project::ApplyOptions,
    intent: Option<&str>,
) -> Result<(ReplayPlan, Vec<Step>), ProjectError> {
    let mut plan = plan_replay(project, recipe, mode, false, actor)?;
    if let Some(w) = intent {
        for d in plan.drafts.iter_mut() {
            d.intent = Some(w.to_string());
        }
    }
    project.record(
        env,
        "recipe.replayed",
        None,
        json!({ "recipe": plan.recipe, "hash": plan.recipe_hash, "mode": plan.mode, "dry_run": false, "diff": plan.diff }),
    )?;
    let opts = crate::project::ApplyOptions {
        plan: true,
        recipe: Some(json!({ "name": plan.recipe, "hash": plan.recipe_hash, "mode": plan.mode })),
        ..opts
    };
    let steps = project.apply(env, plan.drafts.clone(), opts)?;
    Ok((plan, steps))
}

/// Summarise a resolved value for the diff (long arrays are counted, not listed).
pub(crate) fn summarise(v: &Value) -> Value {
    match v {
        Value::Array(a) if a.len() > 8 && a.iter().all(Value::is_number) => {
            json!(format!("{} values", a.len()))
        }
        Value::Array(a) => Value::Array(a.iter().map(summarise).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), summarise(v))).collect())
        }
        other => other.clone(),
    }
}

/// Whether two values are the same as written: a recorded `150` and a
/// computed `150.0` are the same number.
fn same(a: &Value, b: &Value) -> bool {
    a == b || jcs::canonical_bytes(a).ok() == jcs::canonical_bytes(b).ok()
}

pub(crate) fn resolved_diff(
    i: usize,
    op: &str,
    recorded: Option<&Value>,
    new: &Value,
    params: &Value,
) -> Vec<DiffEntry> {
    let mut out = Vec::new();
    let Some(new_map) = new.as_object() else {
        return out;
    };
    for (k, v) in new_map {
        let rec = recorded
            .and_then(|r| r.get(k))
            .cloned()
            .unwrap_or(Value::Null);
        if same(&rec, v) || (rec.is_null() && params.get(k).is_some_and(|p| same(p, v))) {
            continue; // unchanged, or a parameter used exactly as given
        }
        let (recorded, mut new_s) = (summarise(&rec), summarise(v));
        if recorded == new_s {
            // Long arrays summarise alike; say they were re-measured.
            new_s = json!(format!(
                "{} (re-measured)",
                new_s.as_str().unwrap_or("values")
            ));
        }
        out.push(DiffEntry {
            step: i,
            op: op.to_string(),
            param: format!("resolved.{k}"),
            recorded,
            new: new_s,
            reason: "measured on this clip".into(),
        });
    }
    out
}

/// Build the replay of `recipe` on top of `project`'s current stack.
pub fn plan_replay<S: Store>(
    project: &mut Project<S>,
    recipe: &Recipe,
    mode: ReplayMode,
    include_optional: bool,
    actor: Actor,
) -> Result<ReplayPlan, ProjectError> {
    let reg = registry();
    let mut chain = project.render_steps();
    let mut drafts = Vec::new();
    let mut step_indices = Vec::new();
    let mut diff = Vec::new();
    let mut skipped = Vec::new();
    for (i, rs) in recipe.steps.iter().enumerate() {
        if !(rs.enabled || (rs.optional && include_optional)) {
            skipped.push(i);
            continue;
        }
        let op = reg.get(&rs.op, rs.op_version)?;
        let (input, _) = project.render_prefix(&chain)?;
        let mut params = rs.params.clone();
        let mut scope = rs.scope.clone();
        let resolved = match mode {
            ReplayMode::Exact => {
                let r = rs.resolved.clone().ok_or_else(|| {
                    ProjectError::Invalid(format!(
                        "recipe step {i} ({}) has no recorded values; exact replay needs them — use adaptive",
                        rs.op
                    ))
                })?;
                scope
                    .validate(input.duration_s(), input.sample_rate)
                    .map_err(crate::ops::OpError::from)?;
                r
            }
            ReplayMode::Adaptive => {
                let f = project.features_for(&input);
                diff.extend(bindings::apply(
                    i,
                    &rs.op,
                    &rs.bindings,
                    &mut params,
                    &mut scope,
                    &f,
                ));
                let norm = reg.validate(&rs.op, rs.op_version, &params, &scope)?;
                scope
                    .validate(input.duration_s(), input.sample_rate)
                    .map_err(crate::ops::OpError::from)?;
                let r = op.resolve(&norm, &scope, &input)?;
                diff.extend(resolved_diff(i, &rs.op, rs.resolved.as_ref(), &r, &norm));
                params = norm;
                r
            }
        };
        let draft = StepDraft {
            op: rs.op.clone(),
            op_version: Some(rs.op_version),
            params,
            scope: scope.clone(),
            actor: actor.clone(),
            origin: Origin::Recipe,
            intent: rs.intent.clone(),
            rationale: rs.rationale.clone().or_else(|| rs.title.clone()),
            note: None,
            bindings: rs.bindings.clone(),
            resolved: Some(resolved.clone()),
        };
        chain.push(crate::engine::RenderStep {
            op: rs.op.clone(),
            op_version: rs.op_version,
            resolved,
            scope,
        });
        drafts.push(draft);
        step_indices.push(i);
    }
    if drafts.is_empty() {
        return Err(ProjectError::Invalid(
            "the recipe has no enabled steps".into(),
        ));
    }
    Ok(ReplayPlan {
        recipe: recipe.name.clone(),
        recipe_hash: recipe.hash(),
        mode,
        drafts,
        step_indices,
        diff,
        skipped,
    })
}
