// crates/planner/src/dependency.rs
// Step dependency DAG — builds an agent_core::DependencyGraph from LLM specs.
use agent_core::DependencyGraph;
use uuid::Uuid;

use crate::plan::StepSpec;

/// Build a DependencyGraph from LLM-generated step specs.
/// Validates that dependencies reference valid step indices and are not self-references.
pub fn build_dag(specs: &[StepSpec], step_ids: &[Uuid]) -> anyhow::Result<DependencyGraph> {
    let mut graph = DependencyGraph::new();

    for (i, spec) in specs.iter().enumerate() {
        for &dep_idx in &spec.depends {
            if dep_idx >= specs.len() {
                return Err(anyhow::anyhow!(
                    "Invalid dependency: step {i} depends on index {dep_idx} which is out of range ({} steps)",
                    specs.len()
                ));
            }
            if dep_idx == i {
                return Err(anyhow::anyhow!(
                    "Invalid dependency: step {i} depends on itself"
                ));
            }
            graph.add_edge(step_ids[dep_idx], step_ids[i]);
        }
    }

    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_spec(tool: &str, depends: Vec<usize>) -> StepSpec {
        StepSpec {
            tool: tool.to_string(),
            args: serde_json::json!({"cmd": "test"}),
            depends,
            candidates: vec!["fast".into()],
            tool_candidates: vec![],
            delegable: false,
        }
    }

    #[test]
    fn test_valid_dag() {
        let specs = vec![make_spec("bash", vec![]), make_spec("web_search", vec![0])];
        let ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
        let dag = build_dag(&specs, &ids).unwrap();
        let steps = specs
            .iter()
            .enumerate()
            .map(|(i, s)| agent_core::Step {
                id: ids[i],
                tool: s.tool.clone(),
                args: s.args.clone(),
                depends: s.depends.iter().map(|&d| ids[d]).collect(),
                strategy: "fast".into(),
                tool_candidates: vec![],
                delegable: false,
            })
            .collect::<Vec<_>>();
        let layers = dag.topological_layers(&steps);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].len(), 1);
        assert_eq!(layers[1].len(), 1);
    }

    #[test]
    fn test_self_dependency_rejected() {
        let specs = vec![make_spec("bash", vec![0])];
        let ids = vec![Uuid::new_v4()];
        assert!(build_dag(&specs, &ids).is_err());
    }

    #[test]
    fn test_out_of_range_dependency_rejected() {
        let specs = vec![make_spec("bash", vec![5])];
        let ids = vec![Uuid::new_v4()];
        assert!(build_dag(&specs, &ids).is_err());
    }

    #[test]
    fn test_independent_steps_same_layer() {
        let specs = vec![
            make_spec("bash", vec![]),
            make_spec("web_search", vec![]),
            make_spec("bash", vec![]),
        ];
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        let dag = build_dag(&specs, &ids).unwrap();
        let steps = specs
            .iter()
            .enumerate()
            .map(|(i, s)| agent_core::Step {
                id: ids[i],
                tool: s.tool.clone(),
                args: s.args.clone(),
                depends: vec![],
                strategy: "fast".into(),
                tool_candidates: vec![],
                delegable: false,
            })
            .collect::<Vec<_>>();
        let layers = dag.topological_layers(&steps);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].len(), 3);
    }
}
