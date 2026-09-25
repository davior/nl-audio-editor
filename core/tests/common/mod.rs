//! Shared by the integration tests: the JSON Schemas every record must match.

use std::collections::BTreeMap;

use serde_json::Value;

pub fn schemas() -> (boon::Schemas, BTreeMap<&'static str, boon::SchemaIndex>) {
    let base = "https://nlae.local/schemas/";
    let files: [(&str, &str); 7] = [
        (
            "step.schema.json",
            include_str!("../../../schemas/step.schema.json"),
        ),
        (
            "event.schema.json",
            include_str!("../../../schemas/event.schema.json"),
        ),
        (
            "recipe.schema.json",
            include_str!("../../../schemas/recipe.schema.json"),
        ),
        (
            "dataset/step-record.schema.json",
            include_str!("../../../schemas/dataset/step-record.schema.json"),
        ),
        (
            "dataset/episode-record.schema.json",
            include_str!("../../../schemas/dataset/episode-record.schema.json"),
        ),
        (
            "dataset/chat-record.schema.json",
            include_str!("../../../schemas/dataset/chat-record.schema.json"),
        ),
        (
            "dataset/manifest.schema.json",
            include_str!("../../../schemas/dataset/manifest.schema.json"),
        ),
    ];
    let mut compiler = boon::Compiler::new();
    for (name, text) in files {
        compiler
            .add_resource(
                &format!("{base}{name}"),
                serde_json::from_str(text).unwrap(),
            )
            .unwrap();
    }
    let mut schemas = boon::Schemas::new();
    let mut idx = BTreeMap::new();
    for (name, _) in files {
        idx.insert(
            name,
            compiler
                .compile(&format!("{base}{name}"), &mut schemas)
                .unwrap(),
        );
    }
    (schemas, idx)
}

pub fn validate(schemas: &boon::Schemas, idx: boon::SchemaIndex, v: &Value, what: &str) {
    if let Err(e) = schemas.validate(v, idx) {
        panic!(
            "{what} does not match its schema: {e}\n{}",
            serde_json::to_string(v)
                .unwrap()
                .chars()
                .take(600)
                .collect::<String>()
        );
    }
}
