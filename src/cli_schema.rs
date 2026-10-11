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
pub const SCHEMAS: &[(&str, &str)] = &[];

/// The shared envelope as a value.
pub fn envelope_only() -> Value {
    parse(ENVELOPE_SCHEMA)
}

/// The envelope schema for `command`: the shared envelope with
/// `properties.data` replaced by that command's payload schema. `None` when the
/// command has no JSON envelope.
pub fn envelope_for(command: &str) -> Option<Value> {
    let mut envelope = envelope_only();
    envelope["properties"]["data"] = data_schema_for(command)?;
    Some(envelope)
}

/// `forge schema --all`: `{command: schema}`.
pub fn all() -> Value {
    let mut map = serde_json::Map::new();
    for (command, text) in SCHEMAS {
        map.insert((*command).to_string(), parse(text));
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
}
