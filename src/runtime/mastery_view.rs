//! Mastery snapshot accessor — extracted so both the
//! `/__forge/inspect/mastery` HTTP endpoint (issue #304) and the
//! T11.3 proof-run JSON sink (#372) share one read path through the
//! knowledge store.
//!
//! The mastery FSM lives in `workflows/forge-sensei/shared/states.forge`
//! (novice → apprentice → journeyman → expert). Each `swarm_mastery_tuple`
//! agent writes one `learn` entry per level transition under category
//! `mastery-{specialist}-{project}` with content:
//!
//! ```text
//! SWARM-MASTERY specialist:{s} project:{p} level:{l} score:{n}
//! clean:{c} regress:{r} total:{t}
//! last_task:{task_id}
//! ```
//!
//! This module is purely a read-side parser — it never mutates the
//! store. Callers that need a transition timeline use
//! [`collect_all_transitions`]; callers that just need the current
//! level per specialist for one project use [`snapshot_current_levels`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::runtime::knowledge_store::SharedKnowledgeStore;

/// The five canonical dev-cycle specialists tracked by the mastery FSM.
pub const SPECIALISTS: &[&str] = &[
    "planner",
    "implementer",
    "tester",
    "reviewer",
    "release_manager",
];

/// Default level when no mastery entry exists yet for a tuple.
pub const DEFAULT_LEVEL: &str = "novice";

/// One parsed mastery entry. Public so the http_server's transition
/// timeline can reuse the same parse path.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub at: DateTime<Utc>,
    pub specialist: String,
    pub project: String,
    pub level: String,
    pub score: f64,
    pub clean_count: u64,
    pub regress_count: u64,
    pub total: u64,
    pub last_task: String,
}

/// Capture the *current* level for each of the 5 specialists in `project`.
/// Latest entry per (specialist, project) wins. Specialists with no entry
/// default to `DEFAULT_LEVEL` with zero counters — matching the FORGE-side
/// `on start` default in `swarm_mastery_tuple`.
pub fn snapshot_current_levels(
    store: Option<&SharedKnowledgeStore>,
    project: &str,
) -> HashMap<String, Snapshot> {
    let mut result: HashMap<String, Snapshot> = HashMap::new();
    for s in SPECIALISTS {
        result.insert((*s).to_string(), default_snapshot(s, project));
    }
    let all = match collect_all_transitions(store) {
        Some(v) => v,
        None => return result,
    };
    let mut latest: HashMap<String, Snapshot> = HashMap::new();
    for snap in all {
        if snap.project != project {
            continue;
        }
        match latest.get(&snap.specialist) {
            Some(existing) if existing.at >= snap.at => {}
            _ => {
                latest.insert(snap.specialist.clone(), snap);
            }
        }
    }
    for (specialist, snap) in latest {
        if SPECIALISTS.contains(&specialist.as_str()) {
            result.insert(specialist, snap);
        }
    }
    result
}

/// Read every `mastery-*` entry from the knowledge store and return parsed
/// snapshots. Returns `None` only when the store handle is itself absent.
pub fn collect_all_transitions(store: Option<&SharedKnowledgeStore>) -> Option<Vec<Snapshot>> {
    let store = store?;
    let guard = store.lock().ok()?;
    let entries = guard.export_entries();
    drop(guard);

    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(category) = entry.category.as_deref() else {
            continue;
        };
        let Some(rest) = category.strip_prefix("mastery-") else {
            continue;
        };
        let Some(parsed) = parse_swarm_mastery_content(&entry.content) else {
            continue;
        };
        // Prefer the in-content fields (authoritative) but fall back to the
        // category suffix when an individual field is missing.
        let (specialist, project) = if let Some((s, p)) = split_specialist_project(rest) {
            (
                parsed.specialist.clone().unwrap_or_else(|| s.to_string()),
                parsed.project.clone().unwrap_or_else(|| p.to_string()),
            )
        } else {
            (
                parsed.specialist.clone().unwrap_or_default(),
                parsed.project.clone().unwrap_or_default(),
            )
        };
        if specialist.is_empty() || project.is_empty() {
            continue;
        }
        out.push(Snapshot {
            at: entry.created_at,
            specialist,
            project,
            level: parsed.level,
            score: parsed.score,
            clean_count: parsed.clean_count,
            regress_count: parsed.regress_count,
            total: parsed.total,
            last_task: parsed.last_task,
        });
    }
    Some(out)
}

fn default_snapshot(specialist: &str, project: &str) -> Snapshot {
    Snapshot {
        at: Utc::now(),
        specialist: specialist.to_string(),
        project: project.to_string(),
        level: DEFAULT_LEVEL.to_string(),
        score: 0.0,
        clean_count: 0,
        regress_count: 0,
        total: 0,
        last_task: String::new(),
    }
}

#[derive(Default)]
struct ParsedContent {
    specialist: Option<String>,
    project: Option<String>,
    level: String,
    score: f64,
    clean_count: u64,
    regress_count: u64,
    total: u64,
    last_task: String,
}

fn parse_swarm_mastery_content(content: &str) -> Option<ParsedContent> {
    if !content.contains("SWARM-MASTERY") {
        return None;
    }
    let mut parsed = ParsedContent::default();
    for token in content.split_whitespace() {
        let Some((key, value)) = token.split_once(':') else {
            continue;
        };
        match key {
            "specialist" => parsed.specialist = Some(value.to_string()),
            "project" => parsed.project = Some(value.to_string()),
            "level" => parsed.level = value.to_string(),
            "score" => parsed.score = value.parse::<f64>().unwrap_or(0.0),
            "clean" => parsed.clean_count = value.parse::<u64>().unwrap_or(0),
            "regress" => parsed.regress_count = value.parse::<u64>().unwrap_or(0),
            "total" => parsed.total = value.parse::<u64>().unwrap_or(0),
            "last_task" => parsed.last_task = value.to_string(),
            _ => {}
        }
    }
    Some(parsed)
}

fn split_specialist_project(suffix: &str) -> Option<(&str, &str)> {
    for s in SPECIALISTS {
        if let Some(rest) = suffix.strip_prefix(&format!("{}-", s)) {
            return Some((s, rest));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_swarm_mastery_line() {
        let content = "SWARM-MASTERY specialist:planner project:ncmlabs-forge-playground level:journeyman score:0.74 clean:6 regress:1 total:7 last_task:T7";
        let parsed = parse_swarm_mastery_content(content).expect("parses");
        assert_eq!(parsed.specialist.as_deref(), Some("planner"));
        assert_eq!(parsed.project.as_deref(), Some("ncmlabs-forge-playground"));
        assert_eq!(parsed.level, "journeyman");
        assert!((parsed.score - 0.74).abs() < 1e-9);
        assert_eq!(parsed.clean_count, 6);
        assert_eq!(parsed.regress_count, 1);
        assert_eq!(parsed.total, 7);
        assert_eq!(parsed.last_task, "T7");
    }

    #[test]
    fn returns_none_without_marker() {
        assert!(parse_swarm_mastery_content("nothing useful here").is_none());
    }

    #[test]
    fn snapshot_defaults_to_novice_with_no_store() {
        let snap = snapshot_current_levels(None, "ncmlabs-forge-playground");
        assert_eq!(snap.len(), 5);
        for s in SPECIALISTS {
            let entry = snap.get(*s).expect("specialist present");
            assert_eq!(entry.level, DEFAULT_LEVEL);
            assert_eq!(entry.project, "ncmlabs-forge-playground");
        }
    }

    #[test]
    fn splits_specialist_project_suffix() {
        assert_eq!(
            split_specialist_project("planner-ncmlabs-forge-playground"),
            Some(("planner", "ncmlabs-forge-playground"))
        );
        assert_eq!(
            split_specialist_project("release_manager-acme"),
            Some(("release_manager", "acme"))
        );
        assert_eq!(split_specialist_project("not-a-known-prefix"), None);
    }
}
