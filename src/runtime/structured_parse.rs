// Structured-data parsing for FORGE — issue #380.
//
// Given a TOML or JSON document and a FORGE record schema (pulled from a
// `type` declaration via `TypeRegistry`), coerce the document into a
// `Value::Record` whose fields match the schema's declared types. Missing
// fields fall back to type-appropriate defaults; shape mismatches surface
// as descriptive errors that intrinsic handlers pass through to a
// zero-confidence `ConfidentValue` (same contract as `skill.*` failures,
// issue #375).
//
// Nested records recurse through the registry. Lists/arrays match their
// declared element type.

use std::collections::HashMap;

use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::type_registry::{FieldSchema, TypeRegistry};
use crate::types::ForgeType;

/// Parse a TOML document against `schema` resolved through `registry`.
/// Returns a `Value::Record` on success, or a human-readable error string
/// that callers wrap into a zero-confidence value.
pub fn toml_to_record(
    text: &str,
    schema: &FieldSchema,
    registry: &TypeRegistry,
) -> Result<Value, String> {
    let root: toml::Value = toml::from_str(text).map_err(|e| format!("toml parse error: {e}"))?;
    let table = match root {
        toml::Value::Table(t) => t,
        _ => return Err("toml root must be a table".to_string()),
    };
    let generic = toml_table_to_generic(&table);
    coerce_record(&generic, schema, registry, "")
}

/// Parse a JSON document against `schema`. Mirror of `toml_to_record`.
pub fn json_to_record(
    text: &str,
    schema: &FieldSchema,
    registry: &TypeRegistry,
) -> Result<Value, String> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("json parse error: {e}"))?;
    let obj = match root {
        serde_json::Value::Object(m) => m,
        _ => return Err("json root must be an object".to_string()),
    };
    let generic = json_object_to_generic(&obj);
    coerce_record(&generic, schema, registry, "")
}

// ── Internal representation ─────────────────────────────────────
//
// A small dynamic tree keeps the TOML-specific and JSON-specific
// parsing localized to two conversion functions, so `coerce_record`
// can be format-agnostic.

enum Generic {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<Generic>),
    Table(HashMap<String, Generic>),
    Null,
}

fn toml_value_to_generic(v: &toml::Value) -> Generic {
    match v {
        toml::Value::String(s) => Generic::Text(s.clone()),
        toml::Value::Integer(n) => Generic::Number(*n as f64),
        toml::Value::Float(n) => Generic::Number(*n),
        toml::Value::Boolean(b) => Generic::Bool(*b),
        toml::Value::Datetime(d) => Generic::Text(d.to_string()),
        toml::Value::Array(a) => Generic::List(a.iter().map(toml_value_to_generic).collect()),
        toml::Value::Table(t) => Generic::Table(toml_table_to_generic(t)),
    }
}

fn toml_table_to_generic(t: &toml::Table) -> HashMap<String, Generic> {
    t.iter()
        .map(|(k, v)| (k.clone(), toml_value_to_generic(v)))
        .collect()
}

fn json_value_to_generic(v: &serde_json::Value) -> Generic {
    match v {
        serde_json::Value::String(s) => Generic::Text(s.clone()),
        serde_json::Value::Number(n) => Generic::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::Bool(b) => Generic::Bool(*b),
        serde_json::Value::Array(a) => Generic::List(a.iter().map(json_value_to_generic).collect()),
        serde_json::Value::Object(m) => Generic::Table(json_object_to_generic(m)),
        serde_json::Value::Null => Generic::Null,
    }
}

fn json_object_to_generic(
    m: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, Generic> {
    m.iter()
        .map(|(k, v)| (k.clone(), json_value_to_generic(v)))
        .collect()
}

// ── Schema-driven coercion ──────────────────────────────────────

fn coerce_record(
    table: &HashMap<String, Generic>,
    schema: &FieldSchema,
    registry: &TypeRegistry,
    path: &str,
) -> Result<Value, String> {
    let mut fields = HashMap::with_capacity(schema.len());
    for (field_name, field_type) in schema {
        let full_path = if path.is_empty() {
            field_name.clone()
        } else {
            format!("{path}.{field_name}")
        };
        let raw = table.get(field_name);
        let coerced = match raw {
            Some(g) => coerce_value(g, field_type, registry, &full_path)?,
            None => default_for(field_type, registry, &full_path)?,
        };
        fields.insert(field_name.clone(), ConfidentValue::deterministic(coerced));
    }
    Ok(Value::Record(fields))
}

fn coerce_value(
    raw: &Generic,
    ty: &ForgeType,
    registry: &TypeRegistry,
    path: &str,
) -> Result<Value, String> {
    match (ty, raw) {
        (ForgeType::Text, Generic::Text(s)) => Ok(Value::Text(s.clone())),
        (ForgeType::Number, Generic::Number(n)) => Ok(Value::Number(*n)),
        (ForgeType::Bool, Generic::Bool(b)) => Ok(Value::Bool(*b)),
        (ForgeType::Array(inner, _), Generic::List(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let elem_path = format!("{path}[{i}]");
                let v = coerce_value(item, inner, registry, &elem_path)?;
                out.push(ConfidentValue::deterministic(v));
            }
            Ok(Value::Array(out))
        }
        (ForgeType::Named(type_name), Generic::Table(inner_table)) => {
            let nested_schema = registry
                .lookup(type_name)
                .ok_or_else(|| format!("'{path}' references unknown type '{type_name}'"))?;
            coerce_record(inner_table, nested_schema, registry, path)
        }
        // Null / missing → treat same as absent for recoverable defaults.
        (_, Generic::Null) => default_for(ty, registry, path),
        (expected, got) => Err(format!(
            "field '{path}' expected {expected}, got {}",
            describe_generic(got)
        )),
    }
}

fn default_for(ty: &ForgeType, registry: &TypeRegistry, path: &str) -> Result<Value, String> {
    match ty {
        ForgeType::Text => Ok(Value::Text(String::new())),
        ForgeType::Number => Ok(Value::Number(0.0)),
        ForgeType::Bool => Ok(Value::Bool(false)),
        ForgeType::Array(_, _) => Ok(Value::Array(Vec::new())),
        ForgeType::Named(type_name) => {
            // Recurse: build a default record using the nested schema. Missing
            // field → nested defaults, matching serde(default) composition.
            let nested_schema = registry
                .lookup(type_name)
                .ok_or_else(|| format!("'{path}' references unknown type '{type_name}'"))?;
            let empty: HashMap<String, Generic> = HashMap::new();
            coerce_record(&empty, nested_schema, registry, path)
        }
        // Unit / language-only types don't get structured defaults — treat as unsupported.
        other => Err(format!(
            "field '{path}' has unsupported type {other} for structured parsing"
        )),
    }
}

fn describe_generic(g: &Generic) -> &'static str {
    match g {
        Generic::Text(_) => "Text",
        Generic::Number(_) => "Number",
        Generic::Bool(_) => "Bool",
        Generic::List(_) => "List",
        Generic::Table(_) => "Record",
        Generic::Null => "Null",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn registry(src: &str) -> TypeRegistry {
        let program = parse(src).expect("parse");
        TypeRegistry::from_program(&program)
    }

    #[test]
    fn toml_happy_path_flat() {
        let src = r#"type Host
  name: Text
  port: Number
  tls: Bool

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Host").unwrap();
        let toml = r#"
name = "prod"
port = 443
tls  = true
"#;
        let v = toml_to_record(toml, schema, &reg).expect("ok");
        if let Value::Record(fields) = v {
            assert!(matches!(&fields["name"].value, Value::Text(s) if s == "prod"));
            assert!(matches!(&fields["port"].value, Value::Number(n) if *n == 443.0));
            assert!(matches!(&fields["tls"].value, Value::Bool(true)));
        } else {
            panic!("expected record");
        }
    }

    #[test]
    fn toml_missing_field_uses_default() {
        let src = r#"type Cfg
  host: Text
  port: Number

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Cfg").unwrap();
        let v = toml_to_record("host = \"x\"", schema, &reg).unwrap();
        if let Value::Record(fields) = v {
            assert!(matches!(&fields["port"].value, Value::Number(n) if *n == 0.0));
        } else {
            panic!();
        }
    }

    #[test]
    fn toml_wrong_type_errors() {
        let src = r#"type Cfg
  port: Number

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Cfg").unwrap();
        let err = toml_to_record("port = \"not-a-number\"", schema, &reg).unwrap_err();
        assert!(err.contains("port"));
        assert!(err.contains("Number"));
    }

    #[test]
    fn toml_invalid_syntax_errors() {
        let src = r#"type Cfg
  x: Text

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Cfg").unwrap();
        let err = toml_to_record("this is not ::: toml", schema, &reg).unwrap_err();
        assert!(err.contains("toml parse error"));
    }

    #[test]
    fn toml_text_array() {
        let src = r#"type Cfg
  tags: Text[]

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Cfg").unwrap();
        let v = toml_to_record(r#"tags = ["a", "b", "c"]"#, schema, &reg).unwrap();
        if let Value::Record(fields) = v {
            if let Value::Array(items) = &fields["tags"].value {
                assert_eq!(items.len(), 3);
            } else {
                panic!("tags should be Array");
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn toml_nested_record() {
        let src = r#"type Outer
  inner: Inner
  label: Text

type Inner
  count: Number

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Outer").unwrap();
        let toml = r#"
label = "top"

[inner]
count = 7
"#;
        let v = toml_to_record(toml, schema, &reg).unwrap();
        if let Value::Record(fields) = v {
            if let Value::Record(inner) = &fields["inner"].value {
                assert!(matches!(&inner["count"].value, Value::Number(n) if *n == 7.0));
            } else {
                panic!("inner should be a Record");
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn toml_nested_record_missing_uses_nested_defaults() {
        let src = r#"type Outer
  inner: Inner

type Inner
  count: Number
  label: Text

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Outer").unwrap();
        let v = toml_to_record("", schema, &reg).unwrap();
        if let Value::Record(fields) = v {
            if let Value::Record(inner) = &fields["inner"].value {
                assert!(matches!(&inner["count"].value, Value::Number(n) if *n == 0.0));
                assert!(matches!(&inner["label"].value, Value::Text(s) if s.is_empty()));
            } else {
                panic!("inner should default to a Record");
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn unknown_nested_type_errors() {
        let src = r#"type Outer
  inner: DoesNotExist

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Outer").unwrap();
        let err = toml_to_record("", schema, &reg).unwrap_err();
        assert!(err.contains("unknown type"));
    }

    #[test]
    fn json_happy_path_flat() {
        let src = r#"type Host
  name: Text
  port: Number

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Host").unwrap();
        let v = json_to_record(r#"{"name":"prod","port":443}"#, schema, &reg).unwrap();
        if let Value::Record(fields) = v {
            assert!(matches!(&fields["name"].value, Value::Text(s) if s == "prod"));
            assert!(matches!(&fields["port"].value, Value::Number(n) if *n == 443.0));
        } else {
            panic!();
        }
    }

    #[test]
    fn json_null_field_uses_default() {
        let src = r#"type Cfg
  name: Text

fn main
  give "ok"
"#;
        let reg = registry(src);
        let schema = reg.lookup("Cfg").unwrap();
        let v = json_to_record(r#"{"name":null}"#, schema, &reg).unwrap();
        if let Value::Record(fields) = v {
            assert!(matches!(&fields["name"].value, Value::Text(s) if s.is_empty()));
        } else {
            panic!();
        }
    }
}
