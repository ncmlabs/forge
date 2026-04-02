use crate::fleet::spec_model::*;

/// FORGE keywords that cannot be used as identifiers.
const FORGE_KEYWORDS: &[&str] = &[
    "use", "task", "flow", "agent", "pool", "warden", "contract", "system",
    "fn", "needs", "gives", "do", "is", "stage", "if", "else", "when",
    "give", "say", "reason", "classify", "into", "true", "false", "or",
    "with", "escalate", "to", "try", "above", "pure", "event", "states",
    "timer", "endpoint", "type", "requires", "emit", "forward", "subscribe",
    "transition", "start", "cancel", "reset", "for", "in", "not", "and",
    "match",
];

/// Parse a natural-language spec into a SpecModel via keyword extraction.
pub fn parse_spec(spec: &str) -> SpecModel {
    let lower = spec.to_lowercase();
    let system_name = extract_system_name(&lower);
    let agent_names = extract_agent_names(&lower);
    let flow = extract_flow(&lower);
    let capabilities = extract_capabilities(&lower);

    let events = vec![EventSpec {
        name: capitalize_first(&format!("{}Message", capitalize_first(&system_name))),
        fields: vec![
            FieldSpec { name: "sender".into(), type_name: "Text".into() },
            FieldSpec { name: "content".into(), type_name: "Text".into() },
        ],
    }];

    let agents: Vec<AgentSpec> = agent_names
        .iter()
        .map(|name| {
            let states_name = format!("{}Lifecycle", capitalize_first(name));
            AgentSpec {
                name: name.clone(),
                states: Some(StatesSpec {
                    name: states_name,
                    transitions: vec![
                        ("idle".into(), "active".into()),
                        ("active".into(), "done".into()),
                    ],
                }),
                memory_fields: vec![
                    FieldSpec { name: "context".into(), type_name: "Text".into() },
                ],
                handlers: vec![HandlerSpec {
                    event_name: "message".into(),
                    params: vec![
                        FieldSpec { name: "content".into(), type_name: "Text".into() },
                    ],
                    todo_hint: Some(format!("{} handling", name)),
                }],
                subscriptions: vec![],
            }
        })
        .collect();

    let flows = flow.into_iter().collect();

    SpecModel {
        system_name,
        agents,
        flows,
        events,
        types: vec![],
        capabilities,
    }
}

fn extract_system_name(spec: &str) -> String {
    // Try pattern: "a/an <name> system/service/app/platform"
    let stop_words = ["a", "an", "the"];
    let system_words = ["system", "service", "app", "platform", "bot", "tool"];

    let words: Vec<&str> = spec.split_whitespace().collect();

    // Look for "<noun> system/service/..." pattern
    for i in 0..words.len() {
        if system_words.contains(&words[i]) && i > 0 {
            let name = words[i - 1];
            if !stop_words.contains(&name) {
                return sanitize_ident(name);
            }
        }
    }

    // Look for first meaningful word before "with"
    for word in &words {
        if *word == "with" {
            break;
        }
        if !stop_words.contains(word) && !system_words.contains(word) {
            return sanitize_ident(word);
        }
    }

    "generated_system".into()
}

fn extract_agent_names(spec: &str) -> Vec<String> {
    // Look for "with X and Y" or "with X, Y, and Z"
    if let Some(with_pos) = spec.find("with ") {
        let after_with = &spec[with_pos + 5..];
        // Split on "and" or commas
        let parts: Vec<&str> = after_with
            .split(|c: char| c == ',' || c == '.')
            .flat_map(|part| part.split(" and "))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let mut agents = Vec::new();
        for part in parts {
            // Take the last meaningful word(s) from each part
            let name = extract_noun(part);
            if !name.is_empty() {
                agents.push(sanitize_ident(&name));
            }
        }
        if !agents.is_empty() {
            return agents;
        }
    }

    // Fallback: look for role-like words
    let role_words = ["handler", "worker", "processor", "manager", "monitor",
                      "logger", "moderator", "bot", "analyzer", "reporter",
                      "alerter", "checker", "validator", "router", "scheduler"];

    let words: Vec<&str> = spec.split_whitespace().collect();
    let mut agents = Vec::new();
    for word in &words {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if role_words.iter().any(|r| clean.contains(r)) {
            agents.push(sanitize_ident(clean));
        }
    }

    if agents.is_empty() {
        vec!["default_agent".into()]
    } else {
        // Dedup while preserving order
        let mut seen = std::collections::HashSet::new();
        agents.retain(|a| seen.insert(a.clone()));
        agents
    }
}

fn extract_noun(phrase: &str) -> String {
    let stop = ["a", "an", "the", "that", "which", "who", "for", "as"];
    let words: Vec<&str> = phrase
        .split_whitespace()
        .filter(|w| !stop.contains(w))
        .collect();

    // Take the last word as the primary noun
    words.last().copied().unwrap_or("").to_string()
}

fn extract_flow(spec: &str) -> Option<FlowSpec> {
    // Look for "X then Y then Z" pattern
    if spec.contains(" then ") {
        let parts: Vec<&str> = spec.split(" then ").collect();
        if parts.len() >= 2 {
            let mut stages = Vec::new();
            let mut prev: Option<String> = None;

            for part in parts.iter() {
                let name = extract_noun(part.trim());
                if name.is_empty() {
                    continue;
                }
                let stage_name = sanitize_ident(&name);
                let needs = prev.iter().cloned().collect();
                stages.push(StageSpec {
                    name: stage_name.clone(),
                    needs_refs: needs,
                });
                prev = Some(stage_name);
            }

            if stages.len() >= 2 {
                return Some(FlowSpec {
                    name: "pipeline".into(),
                    input: Some(FieldSpec {
                        name: "input".into(),
                        type_name: "Text".into(),
                    }),
                    stages,
                });
            }
        }
    }

    // Look for "pipeline" or "workflow" keywords (word-boundary match for "process")
    let has_pipeline_keyword = spec.contains("pipeline")
        || spec.contains("workflow")
        || spec.split_whitespace().any(|w| w == "process");
    if has_pipeline_keyword {
        return Some(FlowSpec {
            name: "pipeline".into(),
            input: Some(FieldSpec {
                name: "input".into(),
                type_name: "Text".into(),
            }),
            stages: vec![
                StageSpec { name: "intake".into(), needs_refs: vec![] },
                StageSpec { name: "process".into(), needs_refs: vec!["intake".into()] },
            ],
        });
    }

    None
}

fn extract_capabilities(spec: &str) -> Vec<String> {
    let mut caps = vec!["llm.reason".to_string()];

    if spec.contains("classif") || spec.contains("categoriz") || spec.contains("categori") {
        caps.push("llm.classify".to_string());
    }
    if spec.contains("search") || spec.contains("find") || spec.contains("look up") {
        caps.push("web.search".to_string());
    }

    caps
}

fn sanitize_ident(s: &str) -> String {
    // Lowercase, replace non-alphanumeric with underscore
    let clean: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c.to_ascii_lowercase() } else { '_' })
        .collect();

    // Strip leading/trailing underscores
    let trimmed = clean.trim_matches('_').to_string();

    if trimmed.is_empty() {
        return "unnamed".into();
    }

    // Must start with a letter
    let result = if trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("x_{}", trimmed)
    } else {
        trimmed
    };

    // Check against FORGE keywords
    if FORGE_KEYWORDS.contains(&result.as_str()) {
        format!("{}_handler", result)
    } else {
        result
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_system_name() {
        let model = parse_spec("a chat system with moderator and logger");
        assert_eq!(model.system_name, "chat");
    }

    #[test]
    fn extracts_agent_names() {
        let model = parse_spec("a chat system with moderator and logger");
        let names: Vec<&str> = model.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["moderator", "logger"]);
    }

    #[test]
    fn extracts_three_agents() {
        let model = parse_spec("a system with reader, processor, and writer");
        assert_eq!(model.agents.len(), 3);
    }

    #[test]
    fn sanitizes_keyword_names() {
        assert_eq!(sanitize_ident("agent"), "agent_handler");
        assert_eq!(sanitize_ident("task"), "task_handler");
        assert_eq!(sanitize_ident("moderator"), "moderator");
    }

    #[test]
    fn handles_minimal_spec() {
        let model = parse_spec("chatbot");
        assert!(!model.system_name.is_empty());
        assert!(!model.agents.is_empty());
    }

    #[test]
    fn extracts_flow_from_then_pattern() {
        let model = parse_spec("a pipeline that filters then categorizes then archives");
        assert_eq!(model.flows.len(), 1);
        let flow = &model.flows[0];
        assert!(flow.stages.len() >= 2);
    }

    #[test]
    fn capabilities_include_classify() {
        let model = parse_spec("a system that classifies emails");
        assert!(model.capabilities.contains(&"llm.classify".to_string()));
    }

    #[test]
    fn capabilities_include_search() {
        let model = parse_spec("a system that searches the web");
        assert!(model.capabilities.contains(&"web.search".to_string()));
    }
}
