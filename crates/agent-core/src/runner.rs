// crates/agent-core/src/runner.rs
use crate::agent::HermesAgent;
use crate::context::Context;
use anyhow::Result;

/// 运行 agent 并返回它（用于运行后保存会话状态）。
pub async fn run_agent<A: HermesAgent>(mut agent: A, ctx: Context) -> Result<A> {
    let task_desc = ctx.task().unwrap_or("(interactive)");
    tracing::info!(task = task_desc, "Hermes agent starting...");
    agent.run_loop(ctx).await?;
    Ok(agent)
}
