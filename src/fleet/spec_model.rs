/// Intermediate representation of a FORGE system to generate.
///
/// This struct is the Layer 1 → Layer 2 bridge:
/// Layer 1 fills it via keyword extraction,
/// Layer 2 fills it via LLM reasoning.
/// The CodeGen stage is identical for both.

#[derive(Debug, Clone)]
pub struct SpecModel {
    pub system_name: String,
    pub agents: Vec<AgentSpec>,
    pub flows: Vec<FlowSpec>,
    pub events: Vec<EventSpec>,
    pub types: Vec<TypeSpec>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub states: Option<StatesSpec>,
    pub memory_fields: Vec<FieldSpec>,
    pub handlers: Vec<HandlerSpec>,
    pub subscriptions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StatesSpec {
    pub name: String,
    pub transitions: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct HandlerSpec {
    pub event_name: String,
    pub params: Vec<FieldSpec>,
    pub todo_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FlowSpec {
    pub name: String,
    pub input: Option<FieldSpec>,
    pub stages: Vec<StageSpec>,
}

#[derive(Debug, Clone)]
pub struct StageSpec {
    pub name: String,
    pub needs_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EventSpec {
    pub name: String,
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone)]
pub struct TypeSpec {
    pub name: String,
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub name: String,
    pub type_name: String,
}
