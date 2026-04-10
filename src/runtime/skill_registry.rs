// FORGE skill registry — issue #40
// Runtime registry of available skills for host skill bridge.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::runtime::skill::LoadedSkill;
use crate::types::CapabilitySignature;

/// Runtime registry of available skills.
pub struct SkillRegistry {
    skills: HashMap<String, LoadedSkill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    pub fn register(&mut self, skill: LoadedSkill) {
        self.skills.insert(skill.manifest.name.clone(), skill);
    }

    pub fn get(&self, name: &str) -> Option<&LoadedSkill> {
        self.skills.get(name)
    }

    /// Return capability signatures for resolver integration (compile-time validation).
    pub fn capability_signatures(&self) -> HashMap<String, CapabilitySignature> {
        let mut signatures = HashMap::new();
        for (name, skill) in &self.skills {
            if let Some(sig) = &skill.manifest.legacy_signature {
                signatures.insert(format!("skill.{}", name), sig.clone());
            }
            for capability in &skill.manifest.capabilities {
                signatures.insert(
                    format!("skill.{}.{}", name, capability.name),
                    capability.signature.clone(),
                );
            }
        }
        signatures
    }

    pub fn skill_count(&self) -> usize {
        self.skills.len()
    }

    pub fn skill_names(&self) -> Vec<&str> {
        self.skills.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedSkillRegistry = Arc<Mutex<SkillRegistry>>;
