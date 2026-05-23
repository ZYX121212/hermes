// crates/agent-core/src/runner.rs
use anyhow::Result;
use crate::agent::HermesAgent;
use crate::context::Context;

/// Convenience helper to run an agent with a given context.
pub async fn run_agent(mut agent: impl HermesAgent, ctx: Context) -> Result<()> {
    let task_desc = ctx.task().unwrap_or("(interactive)");
    tracing::info!(task = task_desc, "Hermes agent starting...");
    agent.run_loop(ctx).await
}
