// FORGE flow planner: dependency graph + execution waves (issue #10)

use std::collections::{HashMap, HashSet, VecDeque};
use crate::ast::FlowDecl;

#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// stage_name -> set of stage names it depends on
    pub deps: HashMap<String, HashSet<String>>,
    /// all stage names in declaration order
    pub stages: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error("cycle detected in flow dependency graph involving stages: {stages:?}")]
    CycleDetected { stages: Vec<String> },

    #[error("stage `{stage}` references unknown stage `{referenced}`")]
    UnknownStageRef { stage: String, referenced: String },
}

pub struct FlowPlanner;

impl FlowPlanner {
    /// Build a dependency graph from the flow's stage declarations.
    pub fn dependency_graph(flow: &FlowDecl) -> Result<DependencyGraph, PlannerError> {
        let stage_names: HashSet<String> = flow.stages.iter()
            .map(|s| s.node.name.node.clone())
            .collect();

        let mut deps: HashMap<String, HashSet<String>> = HashMap::new();

        for stage in &flow.stages {
            let name = &stage.node.name.node;
            let mut stage_deps = HashSet::new();
            for needs_ref in &stage.node.needs {
                let referenced = &needs_ref.node.stage;
                if !stage_names.contains(referenced) {
                    return Err(PlannerError::UnknownStageRef {
                        stage: name.clone(),
                        referenced: referenced.clone(),
                    });
                }
                stage_deps.insert(referenced.clone());
            }
            deps.insert(name.clone(), stage_deps);
        }

        let stages = flow.stages.iter()
            .map(|s| s.node.name.node.clone())
            .collect();

        Ok(DependencyGraph { deps, stages })
    }

    /// Kahn's algorithm: topological sort into execution waves.
    /// Each wave is a Vec of stage names that can run in parallel.
    pub fn execution_waves(graph: &DependencyGraph) -> Result<Vec<Vec<String>>, PlannerError> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut reverse_deps: HashMap<String, Vec<String>> = HashMap::new();

        for stage in &graph.stages {
            let deg = graph.deps.get(stage).map(|s| s.len()).unwrap_or(0);
            in_degree.insert(stage.clone(), deg);

            if let Some(deps) = graph.deps.get(stage) {
                for dep in deps {
                    reverse_deps.entry(dep.clone()).or_default().push(stage.clone());
                }
            }
        }

        let mut waves = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut processed = 0;

        // Seed with zero in-degree nodes
        for stage in &graph.stages {
            if *in_degree.get(stage).unwrap() == 0 {
                queue.push_back(stage.clone());
            }
        }

        while !queue.is_empty() {
            let wave: Vec<String> = queue.drain(..).collect();
            processed += wave.len();

            for stage in &wave {
                if let Some(dependents) = reverse_deps.get(stage) {
                    for dep in dependents {
                        let deg = in_degree.get_mut(dep).unwrap();
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }

            waves.push(wave);
        }

        if processed != graph.stages.len() {
            let remaining: Vec<String> = graph.stages.iter()
                .filter(|s| *in_degree.get(*s).unwrap() > 0)
                .cloned()
                .collect();
            return Err(PlannerError::CycleDetected { stages: remaining });
        }

        Ok(waves)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;

    fn span<T>(node: T) -> Spanned<T> {
        Spanned { node, span: Span { start: 0, end: 0 } }
    }

    fn make_stage(name: &str, needs: Vec<NeedsRef>) -> Spanned<StageDecl> {
        span(StageDecl {
            name: span(name.to_string()),
            needs: needs.into_iter().map(|n| span(n)).collect(),
            body: vec![],
        })
    }

    fn make_flow(name: &str, stages: Vec<Spanned<StageDecl>>) -> FlowDecl {
        FlowDecl {
            name: span(name.to_string()),
            needs: vec![],
            gives: None,
            stages,
        }
    }

    fn needs_glob(stage: &str) -> NeedsRef {
        NeedsRef { stage: stage.to_string(), field: NeedsRefField::Glob }
    }

    fn needs_field(stage: &str, field: &str) -> NeedsRef {
        NeedsRef { stage: stage.to_string(), field: NeedsRefField::Named(field.to_string()) }
    }

    #[test]
    fn test_no_deps_single_wave() {
        let flow = make_flow("f", vec![
            make_stage("a", vec![]),
            make_stage("b", vec![]),
            make_stage("c", vec![]),
        ]);
        let graph = FlowPlanner::dependency_graph(&flow).unwrap();
        let waves = FlowPlanner::execution_waves(&graph).unwrap();

        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].len(), 3);
    }

    #[test]
    fn test_linear_chain() {
        let flow = make_flow("f", vec![
            make_stage("a", vec![]),
            make_stage("b", vec![needs_glob("a")]),
            make_stage("c", vec![needs_field("b", "x")]),
        ]);
        let graph = FlowPlanner::dependency_graph(&flow).unwrap();
        let waves = FlowPlanner::execution_waves(&graph).unwrap();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["a"]);
        assert_eq!(waves[1], vec!["b"]);
        assert_eq!(waves[2], vec!["c"]);
    }

    #[test]
    fn test_diamond() {
        // A -> B, A -> C, B+C -> D
        let flow = make_flow("f", vec![
            make_stage("a", vec![]),
            make_stage("b", vec![needs_glob("a")]),
            make_stage("c", vec![needs_glob("a")]),
            make_stage("d", vec![needs_glob("b"), needs_glob("c")]),
        ]);
        let graph = FlowPlanner::dependency_graph(&flow).unwrap();
        let waves = FlowPlanner::execution_waves(&graph).unwrap();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["a"]);
        assert!(waves[1].contains(&"b".to_string()));
        assert!(waves[1].contains(&"c".to_string()));
        assert_eq!(waves[2], vec!["d"]);
    }

    #[test]
    fn test_cycle_detection() {
        // A needs B, B needs A
        let flow = make_flow("f", vec![
            make_stage("a", vec![needs_glob("b")]),
            make_stage("b", vec![needs_glob("a")]),
        ]);
        let graph = FlowPlanner::dependency_graph(&flow).unwrap();
        let result = FlowPlanner::execution_waves(&graph);

        assert!(matches!(result, Err(PlannerError::CycleDetected { .. })));
    }

    #[test]
    fn test_unknown_stage_ref() {
        let flow = make_flow("f", vec![
            make_stage("a", vec![needs_glob("nonexistent")]),
        ]);
        let result = FlowPlanner::dependency_graph(&flow);

        assert!(matches!(result, Err(PlannerError::UnknownStageRef { .. })));
    }
}
