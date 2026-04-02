// FORGE agent memory model — issue #11
// HashMap-based field store with context serialization for LLM prompts.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::ast::{FieldDef, Spanned, TypeName};
use crate::runtime::confidence::{ConfidentValue, Value};

/// Agent memory: typed fields initialized from the agent declaration.
#[derive(Debug, Clone)]
pub struct AgentMemory {
    fields: HashMap<String, ConfidentValue>,
}

impl AgentMemory {
    /// Initialize memory from `AgentDecl.memory` field definitions.
    /// Each field gets a type-appropriate default value.
    pub fn new(field_defs: &[Spanned<FieldDef>]) -> Self {
        let mut fields = HashMap::new();
        for fd in field_defs {
            let default = default_for_type(&fd.node.type_name.node);
            fields.insert(fd.node.name.clone(), default);
        }
        Self { fields }
    }

    /// Create empty memory (no fields).
    pub fn empty() -> Self {
        Self { fields: HashMap::new() }
    }

    pub fn get(&self, field: &str) -> Option<&ConfidentValue> {
        self.fields.get(field)
    }

    pub fn set(&mut self, field: &str, value: ConfidentValue) {
        self.fields.insert(field.to_string(), value);
    }

    /// Produce a `Value::Record` for binding as `"memory"` in the executor env.
    pub fn to_record(&self) -> Value {
        let record: HashMap<String, ConfidentValue> = self.fields.clone();
        Value::Record(record)
    }

    /// Serialize memory for LLM prompt context, respecting a rough token budget.
    /// Each ~4 characters ≈ 1 token (rough approximation).
    pub fn to_context_string(&self, budget_tokens: usize) -> String {
        let budget_chars = budget_tokens * 4;
        let mut parts: Vec<String> = Vec::new();
        let mut used = 0;

        for (name, val) in &self.fields {
            let entry = format!("{}: {}", name, val.value);
            if used + entry.len() > budget_chars && !parts.is_empty() {
                break;
            }
            used += entry.len() + 2; // +2 for separator
            parts.push(entry);
        }

        parts.join(", ")
    }

    /// Hash of current memory state for stuck detection.
    pub fn snapshot_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        let mut entries: Vec<(&String, String)> = self.fields.iter()
            .map(|(k, v)| (k, format!("{}", v.value)))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in entries {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Get all field names.
    pub fn field_names(&self) -> Vec<&str> {
        self.fields.keys().map(|s| s.as_str()).collect()
    }
}

/// Produce a type-appropriate default `ConfidentValue`.
fn default_for_type(ty: &TypeName) -> ConfidentValue {
    let value = match ty {
        TypeName::Text | TypeName::Summary | TypeName::Report
        | TypeName::Classification | TypeName::Intent => Value::Text(String::new()),
        TypeName::Number => Value::Number(0.0),
        TypeName::Bool => Value::Bool(false),
        TypeName::Conversation => Value::List(vec![]),
        TypeName::Profile | TypeName::Custom(_) => Value::Record(HashMap::new()),
        TypeName::Results | TypeName::SearchResults => Value::List(vec![]),
        TypeName::Failure => Value::Text(String::new()),
        TypeName::Array(inner, size) => {
            let len = size.unwrap_or(0);
            let element_default = default_for_type(inner);
            Value::Array(vec![element_default; len])
        }
    };
    ConfidentValue::deterministic(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Span, Spanned};

    fn spanned<T>(node: T) -> Spanned<T> {
        Spanned::new(node, Span { start: 0, end: 0 })
    }

    fn text_field(name: &str) -> Spanned<FieldDef> {
        spanned(FieldDef {
            name: name.to_string(),
            type_name: spanned(TypeName::Text),
        })
    }

    fn number_field(name: &str) -> Spanned<FieldDef> {
        spanned(FieldDef {
            name: name.to_string(),
            type_name: spanned(TypeName::Number),
        })
    }

    #[test]
    fn memory_init_defaults() {
        let fields = vec![text_field("topic"), number_field("count")];
        let mem = AgentMemory::new(&fields);
        assert!(matches!(mem.get("topic").unwrap().value, Value::Text(ref s) if s.is_empty()));
        assert!(matches!(mem.get("count").unwrap().value, Value::Number(n) if n == 0.0));
    }

    #[test]
    fn memory_get_set_roundtrip() {
        let fields = vec![text_field("name")];
        let mut mem = AgentMemory::new(&fields);
        mem.set("name", ConfidentValue::deterministic(Value::Text("Alice".into())));
        assert!(matches!(mem.get("name").unwrap().value, Value::Text(ref s) if s == "Alice"));
    }

    #[test]
    fn memory_to_record() {
        let fields = vec![text_field("topic"), number_field("count")];
        let mut mem = AgentMemory::new(&fields);
        mem.set("topic", ConfidentValue::deterministic(Value::Text("billing".into())));
        mem.set("count", ConfidentValue::deterministic(Value::Number(3.0)));
        let rec = mem.to_record();
        match rec {
            Value::Record(map) => {
                assert!(matches!(map["topic"].value, Value::Text(ref s) if s == "billing"));
                assert!(matches!(map["count"].value, Value::Number(n) if n == 3.0));
            }
            _ => panic!("expected Record"),
        }
    }

    #[test]
    fn memory_context_string_budget() {
        let fields = vec![text_field("a"), text_field("b")];
        let mut mem = AgentMemory::new(&fields);
        mem.set("a", ConfidentValue::deterministic(Value::Text("hello".into())));
        mem.set("b", ConfidentValue::deterministic(Value::Text("world".into())));
        // Very small budget — should include at least one field
        let ctx = mem.to_context_string(5);
        assert!(!ctx.is_empty());
    }

    #[test]
    fn memory_snapshot_hash_changes() {
        let fields = vec![number_field("count")];
        let mut mem = AgentMemory::new(&fields);
        let h1 = mem.snapshot_hash();
        mem.set("count", ConfidentValue::deterministic(Value::Number(1.0)));
        let h2 = mem.snapshot_hash();
        assert_ne!(h1, h2);
    }

    #[test]
    fn memory_snapshot_hash_stable() {
        let fields = vec![number_field("count")];
        let mem = AgentMemory::new(&fields);
        assert_eq!(mem.snapshot_hash(), mem.snapshot_hash());
    }
}
