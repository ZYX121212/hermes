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
    /// Fallback tool names to try if the primary tool fails.
    #[serde(default)]
    pub tool_candidates: Vec<String>,
    /// Whether this step can be executed by a sub-agent in parallel.
    #[serde(default)]
    pub delegable: bool,
}
