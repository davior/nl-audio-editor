//! Operation descriptors (`schemas/ops/<id>.v<N>.json`) and parameter
//! validation. Validation runs before any DSP: unknown parameters and values
//! outside the hard limits are rejected (never clamped), and missing values are
//! filled with their defaults so every recorded step has every parameter
//! written out.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::scope::ScopeKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum OpClass {
    Analysis,
    Corrective,
    /// Can only reduce level, cell by cell.
    Attenuative,
    /// Synthesises content; labelled "processed, not factual".
    Reconstruction,
    Creative,
    Safety,
    /// Changes the timeline (removes time, inserts silence); applied to the
    /// output after all processing, never to what processing sees.
    Edit,
}

impl OpClass {
    /// Label carried into records and exports.
    pub fn label(&self) -> &'static str {
        match self {
            OpClass::Analysis => "analysis",
            OpClass::Reconstruction => "reconstruction (processed, not factual)",
            OpClass::Edit => "edited (time removed or inserted)",
            _ => "processed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ResidualKind {
    #[serde(rename = "exact")]
    Exact,
    #[serde(rename = "approximate")]
    Approximate,
    #[serde(rename = "n/a")]
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ParamKind {
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        min: f64,
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
        #[serde(default)]
        auto_allowed: bool,
    },
    Integer {
        min: i64,
        max: i64,
        #[serde(default)]
        auto_allowed: bool,
    },
    Enum {
        values: Vec<String>,
    },
    Bool,
    /// `"auto"` or `{t0, t1}` in seconds.
    TimeRange,
    /// `"auto"` or a list of `{freq_hz, width_hz, depth_db}`.
    Lines {
        max_items: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ParamSpec {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub default: Value,
    #[serde(flatten)]
    pub kind: ParamKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MeasurementSpec {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BindingHint {
    pub param: String,
    pub feature: String,
    pub note: String,
}

/// How the model is shown an operation: always with its full schema
/// (`core`), or in a one-line index, the schema sent when it asks
/// (`on_demand`). Keeps requests small as the catalogue grows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ToolTier {
    Core,
    #[default]
    OnDemand,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Descriptor {
    pub id: String,
    pub version: u32,
    pub title: String,
    pub summary: String,
    pub category: String,
    pub class: OpClass,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub tier: ToolTier,
    pub scopes: Vec<ScopeKind>,
    pub params: Vec<ParamSpec>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Array<Record<string, unknown>>"))]
    pub identity: Vec<Map<String, Value>>,
    pub residual: ResidualKind,
    #[serde(default)]
    pub measurements: Vec<MeasurementSpec>,
    #[serde(default)]
    pub binding_hints: Vec<BindingHint>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("parameter `{param}`: {message}")]
pub struct ParamError {
    pub param: String,
    pub message: String,
}

fn err(param: &str, message: impl Into<String>) -> ParamError {
    ParamError {
        param: param.to_string(),
        message: message.into(),
    }
}

impl Descriptor {
    /// OpenAI-compatible tool definition for the model: the operation's
    /// parameters with their hard limits, and the scopes it accepts.
    pub fn tool_schema(&self) -> Value {
        let mut props = Map::new();
        let mut required = Vec::new();
        for p in &self.params {
            let mut desc = p.title.clone();
            if !p.help.is_empty() {
                desc = format!("{desc}. {}", p.help);
            }
            let auto = serde_json::json!({ "const": "auto" });
            let schema = match &p.kind {
                ParamKind::Number {
                    unit,
                    min,
                    max,
                    auto_allowed,
                    ..
                } => {
                    if let Some(u) = unit {
                        desc = format!("{desc} ({u})");
                    }
                    let n = serde_json::json!({ "type": "number", "minimum": min, "maximum": max });
                    if *auto_allowed {
                        serde_json::json!({ "anyOf": [n, auto] })
                    } else {
                        n
                    }
                }
                ParamKind::Integer {
                    min,
                    max,
                    auto_allowed,
                } => {
                    let n =
                        serde_json::json!({ "type": "integer", "minimum": min, "maximum": max });
                    if *auto_allowed {
                        serde_json::json!({ "anyOf": [n, auto] })
                    } else {
                        n
                    }
                }
                ParamKind::Enum { values } => {
                    serde_json::json!({ "type": "string", "enum": values })
                }
                ParamKind::Bool => serde_json::json!({ "type": "boolean" }),
                ParamKind::TimeRange => serde_json::json!({ "anyOf": [auto, {
                    "type": "object", "required": ["t0", "t1"], "additionalProperties": false,
                    "properties": { "t0": { "type": "number", "minimum": 0 }, "t1": { "type": "number", "minimum": 0 } }
                }] }),
                ParamKind::Lines { max_items } => serde_json::json!({ "anyOf": [auto, {
                    "type": "array", "minItems": 1, "maxItems": max_items,
                    "items": { "type": "object", "required": ["freq_hz", "width_hz", "depth_db"], "additionalProperties": false,
                        "properties": {
                            "freq_hz": { "type": "number", "exclusiveMinimum": 0 },
                            "width_hz": { "type": "number", "exclusiveMinimum": 0, "maximum": 2000 },
                            "depth_db": { "type": "number", "minimum": 0, "maximum": 60 } } }
                }] }),
            };
            let mut schema = schema;
            schema["description"] = Value::String(desc);
            if p.required {
                required.push(Value::String(p.id.clone()));
            }
            props.insert(p.id.clone(), schema);
        }
        let scope_variants: Vec<Value> = self
            .scopes
            .iter()
            .map(|k| match k {
                ScopeKind::Clip => serde_json::json!({ "type": "object", "properties": { "kind": { "const": "clip" } }, "required": ["kind"] }),
                ScopeKind::TimeRange => serde_json::json!({ "type": "object", "required": ["kind", "t0", "t1"],
                    "properties": { "kind": { "const": "time_range" }, "t0": { "type": "number" }, "t1": { "type": "number" } } }),
                ScopeKind::Band => serde_json::json!({ "type": "object", "required": ["kind", "f_lo", "f_hi"],
                    "properties": { "kind": { "const": "band" }, "f_lo": { "type": "number" }, "f_hi": { "type": "number" } } }),
                ScopeKind::TfPatch => serde_json::json!({ "type": "object", "required": ["kind", "t0", "t1", "f_lo", "f_hi"],
                    "properties": { "kind": { "const": "tf_patch" }, "t0": { "type": "number" }, "t1": { "type": "number" },
                        "f_lo": { "type": "number" }, "f_hi": { "type": "number" } } }),
            })
            .collect();
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.id,
                "description": self.summary,
                "parameters": {
                    "type": "object",
                    "required": ["params", "scope"],
                    "additionalProperties": false,
                    "properties": {
                        "params": { "type": "object", "additionalProperties": false, "properties": props, "required": required },
                        "scope": { "oneOf": scope_variants }
                    }
                }
            }
        })
    }

    pub fn param(&self, id: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.id == id)
    }

    /// Validate `params` and fill in defaults. The result has every parameter.
    pub fn normalise_params(&self, params: &Value) -> Result<Value, ParamError> {
        let empty = Map::new();
        let given = match params {
            Value::Null => &empty,
            Value::Object(m) => m,
            _ => return Err(err("(params)", "parameters must be an object")),
        };
        for k in given.keys() {
            if self.param(k).is_none() {
                return Err(err(k, format!("unknown parameter for `{}`", self.id)));
            }
        }
        let mut out = Map::new();
        for spec in &self.params {
            let v = match given.get(&spec.id) {
                Some(v) => v.clone(),
                None if spec.required => return Err(err(&spec.id, "is required")),
                None => spec.default.clone(),
            };
            check_value(spec, &v)?;
            out.insert(spec.id.clone(), v);
        }
        Ok(Value::Object(out))
    }
}

fn check_value(spec: &ParamSpec, v: &Value) -> Result<(), ParamError> {
    let id = spec.id.as_str();
    match &spec.kind {
        ParamKind::Number {
            min,
            max,
            auto_allowed,
            unit,
            ..
        } => {
            if v.as_str() == Some("auto") {
                return if *auto_allowed {
                    Ok(())
                } else {
                    Err(err(id, "does not accept \"auto\""))
                };
            }
            let x = v.as_f64().ok_or_else(|| err(id, "must be a number"))?;
            if !x.is_finite() || x < *min || x > *max {
                let u = unit.as_deref().unwrap_or("");
                return Err(err(
                    id,
                    format!("{x} is outside the hard limits {min}–{max} {u}")
                        .trim_end()
                        .to_string(),
                ));
            }
            Ok(())
        }
        ParamKind::Integer {
            min,
            max,
            auto_allowed,
        } => {
            if v.as_str() == Some("auto") {
                return if *auto_allowed {
                    Ok(())
                } else {
                    Err(err(id, "does not accept \"auto\""))
                };
            }
            let x = v.as_i64().ok_or_else(|| err(id, "must be an integer"))?;
            if x < *min || x > *max {
                return Err(err(
                    id,
                    format!("{x} is outside the hard limits {min}–{max}"),
                ));
            }
            Ok(())
        }
        ParamKind::Enum { values } => {
            let s = v.as_str().ok_or_else(|| err(id, "must be a string"))?;
            if values.iter().any(|x| x == s) {
                Ok(())
            } else {
                Err(err(
                    id,
                    format!("\"{s}\" is not one of {}", values.join(", ")),
                ))
            }
        }
        ParamKind::Bool => v
            .as_bool()
            .map(|_| ())
            .ok_or_else(|| err(id, "must be true or false")),
        ParamKind::TimeRange => {
            if v.as_str() == Some("auto") {
                return Ok(());
            }
            let t0 = v.get("t0").and_then(Value::as_f64);
            let t1 = v.get("t1").and_then(Value::as_f64);
            let extra = v
                .as_object()
                .map_or(true, |m| m.keys().any(|k| k != "t0" && k != "t1"));
            match (t0, t1) {
                (Some(a), Some(b)) if !extra && a >= 0.0 && b > a && b.is_finite() => Ok(()),
                _ => Err(err(
                    id,
                    "must be \"auto\" or {\"t0\": seconds, \"t1\": seconds} with t1 > t0 ≥ 0",
                )),
            }
        }
        ParamKind::Lines { max_items } => {
            if v.as_str() == Some("auto") {
                return Ok(());
            }
            let items = v
                .as_array()
                .ok_or_else(|| err(id, "must be \"auto\" or a list of lines"))?;
            if items.is_empty() || items.len() > *max_items {
                return Err(err(
                    id,
                    format!("must list between 1 and {max_items} lines"),
                ));
            }
            for (i, it) in items.iter().enumerate() {
                let m = it
                    .as_object()
                    .ok_or_else(|| err(id, format!("line {i} must be an object")))?;
                if m.keys()
                    .any(|k| !matches!(k.as_str(), "freq_hz" | "width_hz" | "depth_db"))
                {
                    return Err(err(id, format!("line {i} has an unknown field")));
                }
                let f = m.get("freq_hz").and_then(Value::as_f64).unwrap_or(-1.0);
                let w = m.get("width_hz").and_then(Value::as_f64).unwrap_or(-1.0);
                let d = m.get("depth_db").and_then(Value::as_f64).unwrap_or(-1.0);
                if !(f > 0.0 && f <= 96000.0) {
                    return Err(err(id, format!("line {i}: freq_hz must be in 0–96000")));
                }
                if !(w > 0.0 && w <= 2000.0) {
                    return Err(err(id, format!("line {i}: width_hz must be in 0–2000")));
                }
                if !(0.0..=60.0).contains(&d) {
                    return Err(err(id, format!("line {i}: depth_db must be in 0–60")));
                }
            }
            Ok(())
        }
    }
}
