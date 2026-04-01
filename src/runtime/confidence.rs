// FORGE confidence model — issue #7
// Every runtime value carries confidence metadata.

use std::collections::HashMap;
use std::fmt;

use crate::types::ConfidenceSource;

// ── Value ───────────────────────────────────────────────────

/// Runtime value representation for FORGE.
#[derive(Debug, Clone)]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
    Unit,
    List(Vec<ConfidentValue>),
    Record(HashMap<String, ConfidentValue>),
    Array(Vec<ConfidentValue>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Text(s) => write!(f, "{s}"),
            Value::Number(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Unit => write!(f, "()"),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item.value)?;
                }
                write!(f, "]")
            }
            Value::Record(fields) => {
                write!(f, "{{")?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {}", v.value)?;
                }
                write!(f, "}}")
            }
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item.value)?;
                }
                write!(f, "]")
            }
        }
    }
}

// ── ConfidentValue ──────────────────────────────────────────

/// A runtime value paired with confidence metadata.
#[derive(Debug, Clone)]
pub struct ConfidentValue {
    pub value: Value,
    pub confidence: f32,
    pub source: ConfidenceSource,
}

// ── Predicates ──────────────────────────────────────────────

impl ConfidentValue {
    /// High confidence: >= 0.8
    pub fn sure(&self) -> bool {
        self.confidence >= 0.8
    }

    /// Confidence >= custom threshold
    pub fn sure_above(&self, threshold: f32) -> bool {
        self.confidence >= threshold
    }

    /// Medium confidence: 0.5 <= c < 0.8
    pub fn unsure(&self) -> bool {
        self.confidence >= 0.5 && self.confidence < 0.8
    }

    /// Low confidence: < 0.5
    pub fn unreliable(&self) -> bool {
        self.confidence < 0.5
    }

    /// Contradictory signals: ConsensusAgreement source with agreement < 0.6
    pub fn conflicted(&self) -> bool {
        matches!(self.source, ConfidenceSource::ConsensusAgreement(a) if a < 0.6)
    }
}

// ── Constructors ────────────────────────────────────────────

fn clamp_confidence(c: f32) -> f32 {
    c.clamp(0.0, 1.0)
}

impl ConfidentValue {
    /// Create a deterministic value (from `pure` functions) — confidence is always 1.0.
    pub fn deterministic(value: Value) -> Self {
        Self {
            value,
            confidence: 1.0,
            source: ConfidenceSource::Deterministic,
        }
    }

    /// Create from LLM output with a confidence score.
    pub fn from_llm(value: Value, confidence: f32) -> Self {
        let confidence = clamp_confidence(confidence);
        Self {
            value,
            confidence,
            source: ConfidenceSource::LLMDirect(confidence),
        }
    }

    /// Create from consensus of multiple agents.
    pub fn from_consensus(value: Value, agreement: f32) -> Self {
        let agreement = clamp_confidence(agreement);
        Self {
            value,
            confidence: agreement,
            source: ConfidenceSource::ConsensusAgreement(agreement),
        }
    }

    /// Derive confidence from an upstream value (propagation).
    pub fn derived(value: Value, upstream_confidence: f32) -> Self {
        let confidence = clamp_confidence(upstream_confidence);
        Self {
            value,
            confidence,
            source: ConfidenceSource::Derived(confidence),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn text_val(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    // -- sure() boundary tests --

    #[test]
    fn sure_at_0_79_is_false() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.79);
        assert!(!cv.sure());
    }

    #[test]
    fn sure_at_0_8_is_true() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.8);
        assert!(cv.sure());
    }

    #[test]
    fn sure_at_1_0_is_true() {
        let cv = ConfidentValue::deterministic(text_val("x"));
        assert!(cv.sure());
    }

    // -- sure_above() --

    #[test]
    fn sure_above_custom_threshold() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.94);
        assert!(!cv.sure_above(0.95));

        let cv = ConfidentValue::from_llm(text_val("x"), 0.95);
        assert!(cv.sure_above(0.95));
    }

    // -- unsure() range --

    #[test]
    fn unsure_below_0_5_is_false() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.49);
        assert!(!cv.unsure());
    }

    #[test]
    fn unsure_at_0_5_is_true() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.5);
        assert!(cv.unsure());
    }

    #[test]
    fn unsure_at_0_79_is_true() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.79);
        assert!(cv.unsure());
    }

    #[test]
    fn unsure_at_0_8_is_false() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.8);
        assert!(!cv.unsure());
    }

    // -- unreliable() --

    #[test]
    fn unreliable_at_0_5_is_false() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.5);
        assert!(!cv.unreliable());
    }

    #[test]
    fn unreliable_at_0_49_is_true() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.49);
        assert!(cv.unreliable());
    }

    #[test]
    fn unreliable_at_0_is_true() {
        let cv = ConfidentValue::from_llm(text_val("x"), 0.0);
        assert!(cv.unreliable());
    }

    // -- conflicted() --

    #[test]
    fn conflicted_only_for_consensus_below_0_6() {
        // ConsensusAgreement with agreement < 0.6 → conflicted
        let cv = ConfidentValue::from_consensus(text_val("x"), 0.59);
        assert!(cv.conflicted());

        // ConsensusAgreement with agreement >= 0.6 → not conflicted
        let cv = ConfidentValue::from_consensus(text_val("x"), 0.6);
        assert!(!cv.conflicted());

        // LLMDirect with low confidence → NOT conflicted (wrong source)
        let cv = ConfidentValue::from_llm(text_val("x"), 0.3);
        assert!(!cv.conflicted());

        // Deterministic → NOT conflicted
        let cv = ConfidentValue::deterministic(text_val("x"));
        assert!(!cv.conflicted());
    }

    // -- deterministic constructor --

    #[test]
    fn deterministic_has_confidence_1() {
        let cv = ConfidentValue::deterministic(Value::Number(42.0));
        assert_eq!(cv.confidence, 1.0);
        assert_eq!(cv.source, ConfidenceSource::Deterministic);
    }

    // -- clamping --

    #[test]
    fn confidence_clamped_to_0_1() {
        let cv = ConfidentValue::from_llm(text_val("x"), 1.5);
        assert_eq!(cv.confidence, 1.0);

        let cv = ConfidentValue::from_llm(text_val("x"), -0.3);
        assert_eq!(cv.confidence, 0.0);

        let cv = ConfidentValue::from_consensus(text_val("x"), 2.0);
        assert_eq!(cv.confidence, 1.0);

        let cv = ConfidentValue::derived(text_val("x"), -1.0);
        assert_eq!(cv.confidence, 0.0);
    }

    // -- nested values --

    #[test]
    fn list_contains_confident_values() {
        let list = Value::List(vec![
            ConfidentValue::deterministic(Value::Number(1.0)),
            ConfidentValue::from_llm(Value::Number(2.0), 0.7),
        ]);
        let cv = ConfidentValue::deterministic(list);
        if let Value::List(items) = &cv.value {
            assert_eq!(items.len(), 2);
            assert!(items[0].sure());
            assert!(items[1].unsure());
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn record_contains_confident_values() {
        let mut fields = HashMap::new();
        fields.insert(
            "name".to_string(),
            ConfidentValue::from_llm(Value::Text("Alice".to_string()), 0.9),
        );
        let cv = ConfidentValue::deterministic(Value::Record(fields));
        if let Value::Record(f) = &cv.value {
            assert!(f["name"].sure());
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn array_contains_confident_values() {
        let arr = Value::Array(vec![
            ConfidentValue::from_llm(Value::Text("X".to_string()), 0.85),
            ConfidentValue::from_llm(Value::Text("O".to_string()), 0.4),
        ]);
        let cv = ConfidentValue::deterministic(arr);
        if let Value::Array(items) = &cv.value {
            assert!(items[0].sure());
            assert!(items[1].unreliable());
        } else {
            panic!("expected Array");
        }
    }

    // -- display --

    #[test]
    fn display_values() {
        assert_eq!(format!("{}", Value::Text("hello".into())), "hello");
        assert_eq!(format!("{}", Value::Number(3.14)), "3.14");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Unit), "()");
    }
}
