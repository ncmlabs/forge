//! CLI introspection: the JSON Schemas behind `forge schema` and the metadata
//! behind `forge help --json` (#476).
//!
//! Every in-scope command builds its `data` payload ad hoc as a
//! `serde_json::Value` at the call site (there are no Rust types to derive a
//! schema from), so the schemas are hand-written, one file per command under
//! `schemas/`, and embedded here with `include_str!`. `tests/cli_schema_tests.rs`
//! is the drift guard: it runs each command for real and validates the emitted
//! envelope against `forge schema <command>`, so a payload that gains, renames
//! or retypes a key fails the suite until the schema is updated.

use serde_json::Value;

/// The shared envelope (`schemas/envelope.schema.json`), `data` left generic.
/// Bare `forge schema` prints this document.
pub const ENVELOPE_SCHEMA: &str = include_str!("../schemas/envelope.schema.json");

/// One `<command>.data.schema.json` per envelope-producing command, in the
/// order `forge schema --all` prints them. Commands without a JSON contract
/// (`serve`, `agent`, `agent-inspect`, `fleet` — see `docs/forge-reference.md`
/// §27) and the introspection commands themselves (`schema`, `help`) are absent
/// on purpose: `json_supported` is derived from this table.
pub const SCHEMAS: &[(&str, &str)] = &[
    ("parse", include_str!("../schemas/parse.data.schema.json")),
    ("check", include_str!("../schemas/check.data.schema.json")),
    (
        "explain",
        include_str!("../schemas/explain.data.schema.json"),
    ),
    ("run", include_str!("../schemas/run.data.schema.json")),
    ("trace", include_str!("../schemas/trace.data.schema.json")),
    ("test", include_str!("../schemas/test.data.schema.json")),
    ("cost", include_str!("../schemas/cost.data.schema.json")),
    ("build", include_str!("../schemas/build.data.schema.json")),
    ("send", include_str!("../schemas/send.data.schema.json")),
    ("wake", include_str!("../schemas/wake.data.schema.json")),
    ("store", include_str!("../schemas/store.data.schema.json")),
    ("export", include_str!("../schemas/export.data.schema.json")),
    ("import", include_str!("../schemas/import.data.schema.json")),
    (
        "inspect",
        include_str!("../schemas/inspect.data.schema.json"),
    ),
    ("init", include_str!("../schemas/init.data.schema.json")),
];

/// The shared envelope as a value.
pub fn envelope_only() -> Value {
    parse(ENVELOPE_SCHEMA)
}

/// The envelope schema for `command`: the shared envelope with
/// `properties.data` replaced by that command's payload schema. `None` when the
/// command has no JSON envelope.
///
/// `data` accepts `{}` as well as the payload: a command that fails before it
/// builds one (`out.fail` — io, config and runtime failures) emits an error
/// envelope with empty data, so the escape hatch is part of the contract rather
/// than a variant each payload schema repeats.
pub fn envelope_for(command: &str) -> Option<Value> {
    let data = data_schema_for(command)?;
    let mut envelope = envelope_only();
    envelope["properties"]["data"] = serde_json::json!({
        "description": format!(
            "`{command}` payload, or `{{}}` when the command failed before building one."
        ),
        "anyOf": [data, { "title": "no payload", "type": "object", "maxProperties": 0 }]
    });
    Some(envelope)
}

/// `forge schema --all`: `{command: schema}`, each value exactly what
/// `forge schema <command>` prints.
pub fn all() -> Value {
    let mut map = serde_json::Map::new();
    for (command, _) in SCHEMAS {
        if let Some(schema) = envelope_for(command) {
            map.insert((*command).to_string(), schema);
        }
    }
    Value::Object(map)
}

/// One command's `data` payload schema.
pub fn data_schema_for(command: &str) -> Option<Value> {
    SCHEMAS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, text)| parse(text))
}

/// Every command that has a data schema, in table order.
pub fn commands() -> Vec<&'static str> {
    SCHEMAS.iter().map(|(name, _)| *name).collect()
}

/// Parse an embedded schema. Malformed JSON is a build-time mistake, caught by
/// the unit tests below rather than at a call site.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("embedded schema is not JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_schema_has_the_envelope_contract() {
        let envelope = envelope_only();
        assert_eq!(
            envelope["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        let required = envelope["required"].as_array().expect("required array");
        for key in [
            "status",
            "command",
            "data",
            "context",
            "next_steps",
            "warnings",
            "cost",
            "error",
            "duration_ms",
        ] {
            assert!(
                required.iter().any(|r| r == key),
                "envelope must require {key}: {required:?}"
            );
            assert!(
                envelope["properties"][key].is_object(),
                "envelope must describe {key}"
            );
        }
        assert_eq!(envelope["additionalProperties"], false);
        assert!(envelope["$defs"]["error"].is_object());
    }

    #[test]
    fn command_names_are_unique_and_have_no_duplicates() {
        let mut names = commands();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate command in SCHEMAS");
    }

    #[test]
    fn every_data_schema_is_a_json_schema_for_an_object() {
        for (command, text) in SCHEMAS {
            let schema = parse(text);
            assert_eq!(
                schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
                "{command} must declare the 2020-12 dialect"
            );
            assert_eq!(schema["type"], "object", "{command} data is an object");
            assert!(schema["title"].is_string(), "{command} needs a title");
            assert!(
                schema["description"].is_string(),
                "{command} needs a description"
            );
        }
    }

    /// A data schema's `$ref`s resolve inside the composed document, whose
    /// `$defs` come from the envelope. A typo would otherwise only surface as a
    /// validation failure in the drift guard.
    #[test]
    fn data_schema_refs_resolve_inside_the_envelope() {
        let envelope = envelope_only();
        for (command, text) in SCHEMAS {
            let mut refs = Vec::new();
            collect_refs(&parse(text), &mut refs);
            for r in refs {
                let name = r
                    .strip_prefix("#/$defs/")
                    .unwrap_or_else(|| panic!("{command}: `{r}` is not a local $defs reference"));
                assert!(
                    envelope["$defs"][name].is_object(),
                    "{command}: dangling $ref `{r}`"
                );
            }
        }
    }

    fn collect_refs(value: &Value, refs: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if key == "$ref" {
                        if let Some(r) = value.as_str() {
                            refs.push(r.to_string());
                        }
                    }
                    collect_refs(value, refs);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_refs(item, refs);
                }
            }
            _ => {}
        }
    }
}
