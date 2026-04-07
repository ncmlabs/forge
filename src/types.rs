// FORGE type system
// See issue #6 (resolver) and #7 (ConfidentValue) for full implementation

use crate::ast::TypeName;

// ── ForgeType ────────────────────────────────────────────────

/// Semantic type used during checking (mirrors ast::TypeName but adds Unit, Embedding, Duration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeType {
    Text,
    Number,
    Bool,
    Unit,
    Results,
    Report,
    Intent,
    Classification,
    Summary,
    Failure,
    Conversation,
    Profile,
    SearchResults,
    Request,
    Response,
    Headers,
    Html,
    Embedding,
    Duration,
    Named(String),
    Array(Box<ForgeType>, Option<usize>),
}

impl std::fmt::Display for ForgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForgeType::Text => write!(f, "Text"),
            ForgeType::Number => write!(f, "Number"),
            ForgeType::Bool => write!(f, "Bool"),
            ForgeType::Unit => write!(f, "Unit"),
            ForgeType::Results => write!(f, "Results"),
            ForgeType::Report => write!(f, "Report"),
            ForgeType::Intent => write!(f, "Intent"),
            ForgeType::Classification => write!(f, "Classification"),
            ForgeType::Summary => write!(f, "Summary"),
            ForgeType::Failure => write!(f, "Failure"),
            ForgeType::Conversation => write!(f, "Conversation"),
            ForgeType::Profile => write!(f, "Profile"),
            ForgeType::SearchResults => write!(f, "SearchResults"),
            ForgeType::Request => write!(f, "Request"),
            ForgeType::Response => write!(f, "Response"),
            ForgeType::Headers => write!(f, "Headers"),
            ForgeType::Html => write!(f, "Html"),
            ForgeType::Embedding => write!(f, "Embedding"),
            ForgeType::Duration => write!(f, "Duration"),
            ForgeType::Named(s) => write!(f, "{s}"),
            ForgeType::Array(inner, Some(n)) => write!(f, "{inner}[{n}]"),
            ForgeType::Array(inner, None) => write!(f, "{inner}[]"),
        }
    }
}

// ── Capability signature ─────────────────────────────────────

/// Typed signature for a built-in capability (what the registry stores).
#[derive(Debug, Clone)]
pub struct CapabilitySignature {
    pub inputs: Vec<ForgeType>,
    pub output: ForgeType,
}

// ── Confidence source ────────────────────────────────────────

/// Tags how a value's confidence was determined.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ConfidenceSource {
    /// Pure function — always 1.0
    Deterministic,
    /// From model heuristic
    LLMDirect(f32),
    /// Multiple agents agreed
    ConsensusAgreement(f32),
    /// Propagated from upstream
    Derived(f32),
    /// Retrieved from knowledge store (confidence = retrieval relevance)
    KnowledgeRecall(f32),
    /// Imported from an external agent package (confidence = capped value)
    ImportedKnowledge(f32),
    /// From direct CLI execution — always uncertain (external process)
    ExecResult(f32),
    /// From an external skill invocation (host-provided capability)
    SkillInvocation(f32),
}

// ── Conversions ──────────────────────────────────────────────

/// Convert an AST TypeName to a semantic ForgeType.
pub fn from_type_name(tn: &TypeName) -> ForgeType {
    match tn {
        TypeName::Text => ForgeType::Text,
        TypeName::Number => ForgeType::Number,
        TypeName::Bool => ForgeType::Bool,
        TypeName::Results => ForgeType::Results,
        TypeName::Report => ForgeType::Report,
        TypeName::Intent => ForgeType::Intent,
        TypeName::Summary => ForgeType::Summary,
        TypeName::Failure => ForgeType::Failure,
        TypeName::Classification => ForgeType::Classification,
        TypeName::Conversation => ForgeType::Conversation,
        TypeName::Profile => ForgeType::Profile,
        TypeName::SearchResults => ForgeType::SearchResults,
        TypeName::Request => ForgeType::Request,
        TypeName::Response => ForgeType::Response,
        TypeName::Headers => ForgeType::Headers,
        TypeName::Html => ForgeType::Html,
        TypeName::Custom(s) => ForgeType::Named(s.clone()),
        TypeName::Array(inner, size) => ForgeType::Array(Box::new(from_type_name(inner)), *size),
    }
}

// ── Type compatibility ───────────────────────────────────────

/// POC permissive compatibility check for the `>>` composition operator.
/// - Text is compatible with anything (permissive rule from issue #6)
/// - Same type is compatible
/// - Arrays match ignoring size
pub fn is_compatible(from: &ForgeType, to: &ForgeType) -> bool {
    if from == to {
        return true;
    }
    // POC: Text >> anything always works
    if *from == ForgeType::Text {
        return true;
    }
    // Html is compatible with Text (Html can flow where Text is expected)
    if *from == ForgeType::Html && *to == ForgeType::Text {
        return true;
    }
    // Arrays: compatible if element types match (ignore size)
    if let (ForgeType::Array(a, _), ForgeType::Array(b, _)) = (from, to) {
        return is_compatible(a, b);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_type_is_compatible() {
        assert!(is_compatible(&ForgeType::Number, &ForgeType::Number));
        assert!(is_compatible(&ForgeType::Bool, &ForgeType::Bool));
    }

    #[test]
    fn text_compatible_with_anything() {
        assert!(is_compatible(&ForgeType::Text, &ForgeType::Number));
        assert!(is_compatible(&ForgeType::Text, &ForgeType::Classification));
        assert!(is_compatible(
            &ForgeType::Text,
            &ForgeType::Named("Foo".into())
        ));
    }

    #[test]
    fn different_types_incompatible() {
        assert!(!is_compatible(&ForgeType::Number, &ForgeType::Bool));
        assert!(!is_compatible(
            &ForgeType::Classification,
            &ForgeType::Results
        ));
    }

    #[test]
    fn arrays_compatible_ignoring_size() {
        let a = ForgeType::Array(Box::new(ForgeType::Text), Some(9));
        let b = ForgeType::Array(Box::new(ForgeType::Text), None);
        assert!(is_compatible(&a, &b));

        // Text[9] is compatible with Number[3] because Text >> anything (POC rule)
        let c = ForgeType::Array(Box::new(ForgeType::Number), Some(3));
        assert!(is_compatible(&a, &c));

        // But Number[3] is not compatible with Text[]
        let d = ForgeType::Array(Box::new(ForgeType::Number), Some(3));
        let e = ForgeType::Array(Box::new(ForgeType::Bool), None);
        assert!(!is_compatible(&d, &e));
    }

    #[test]
    fn from_type_name_converts() {
        assert_eq!(from_type_name(&TypeName::Text), ForgeType::Text);
        assert_eq!(from_type_name(&TypeName::Number), ForgeType::Number);
        assert_eq!(
            from_type_name(&TypeName::Custom("Player".into())),
            ForgeType::Named("Player".into())
        );
        assert_eq!(
            from_type_name(&TypeName::Array(Box::new(TypeName::Text), Some(9))),
            ForgeType::Array(Box::new(ForgeType::Text), Some(9))
        );
    }

    #[test]
    fn display_types() {
        assert_eq!(ForgeType::Text.to_string(), "Text");
        assert_eq!(ForgeType::Named("Foo".into()).to_string(), "Foo");
        assert_eq!(
            ForgeType::Array(Box::new(ForgeType::Text), Some(9)).to_string(),
            "Text[9]"
        );
        assert_eq!(
            ForgeType::Array(Box::new(ForgeType::Number), None).to_string(),
            "Number[]"
        );
    }
}
