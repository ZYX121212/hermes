// crates/planner/src/plan.rs
// Planner-specific types for LLM interaction.
// Core types (Plan, Step, DependencyGraph) are re-exported from agent-core.
use serde::Deserialize;

/// Raw step specification parsed from LLM JSON output.
/// The `depends` field holds 0-based indices into the step array.
#[derive(Debug, Clone, Deserialize)]
pub struct StepSpec {
    pub tool: String,
    pub args: serde_json::Value,
    #[serde(default)]
    pub depends: Vec<usize>,
    #[serde(default)]
    pub candidates: Vec<String>,
}
