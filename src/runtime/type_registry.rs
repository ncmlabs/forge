// Runtime type registry — issue #380.
//
// Indexes every `type` declaration in the loaded Program so that intrinsics
// like `toml.parse` / `json.parse` can coerce a dynamic document into a
// typed FORGE record at runtime. The checker already resolves types at
// compile time; the registry surfaces the same information to intrinsic
// handlers that need it during evaluation.

use std::collections::HashMap;

use crate::ast::{Program, TopLevel};
use crate::types::{from_type_name, ForgeType};

/// Ordered field list for a record type: (field_name, declared_type).
pub type FieldSchema = Vec<(String, ForgeType)>;

/// Map from user-declared type name to its field schema.
#[derive(Debug, Default, Clone)]
pub struct TypeRegistry {
    types: HashMap<String, FieldSchema>,
}

impl TypeRegistry {
    /// Scan a Program for every `type` decl and index its fields.
    pub fn from_program(program: &Program) -> Self {
        let mut types = HashMap::new();
        for item in &program.items {
            if let TopLevel::TypeDef(td) = &item.node {
                let schema: FieldSchema = td
                    .fields
                    .iter()
                    .map(|f| (f.node.name.clone(), from_type_name(&f.node.type_name.node)))
                    .collect();
                types.insert(td.name.node.clone(), schema);
            }
        }
        Self { types }
    }

    pub fn lookup(&self, name: &str) -> Option<&FieldSchema> {
        self.types.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.types.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn empty_program_has_empty_registry() {
        let program = parse("fn main\n  give \"hi\"\n").expect("parse");
        let reg = TypeRegistry::from_program(&program);
        assert!(reg.lookup("Anything").is_none());
    }

    #[test]
    fn indexes_single_type_decl() {
        let src = r#"type Person
  name: Text
  age: Number

fn main
  give "ok"
"#;
        let program = parse(src).expect("parse");
        let reg = TypeRegistry::from_program(&program);
        let schema = reg.lookup("Person").expect("Person should be indexed");
        assert_eq!(schema.len(), 2);
        assert_eq!(schema[0].0, "name");
        assert_eq!(schema[0].1, ForgeType::Text);
        assert_eq!(schema[1].0, "age");
        assert_eq!(schema[1].1, ForgeType::Number);
    }

    #[test]
    fn indexes_multiple_type_decls() {
        let src = r#"type A
  x: Text

type B
  y: Number
  z: Bool

fn main
  give "ok"
"#;
        let program = parse(src).expect("parse");
        let reg = TypeRegistry::from_program(&program);
        assert!(reg.contains("A"));
        assert!(reg.contains("B"));
        assert_eq!(reg.lookup("B").unwrap().len(), 2);
    }

    #[test]
    fn preserves_field_order() {
        let src = r#"type Ordered
  first: Text
  second: Text
  third: Text

fn main
  give "ok"
"#;
        let program = parse(src).expect("parse");
        let reg = TypeRegistry::from_program(&program);
        let schema = reg.lookup("Ordered").unwrap();
        let names: Vec<&str> = schema.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["first", "second", "third"]);
    }
}
