// crates/scheduler/src/subagent.rs
// Trait and types for sub-agent delegation, decoupled from the agent
// implementation to avoid circular dependencies (scheduler ← hermess-agent).

/// Output from a delegated sub-agent run.
#[derive(Debug, Clone)]
pub struct SubAgentOutput {
    pub summary: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Runner that spawns an independent sub-agent for a delegable step.
/// Implemented by hermess-agent and passed to Scheduler via builder.
#[async_trait::async_trait]
pub trait SubAgentRunner: Send + Sync {
    /// Run a sub-agent to autonomously complete the given task description.
    async fn run(&self, task: &str) -> anyhow::Result<SubAgentOutput>;
}
