// crates/agent-core/src/agent.rs
use crate::context::Context;
use crate::{ExecutionResult, Insight, Observation, Plan};
use anyhow::Result;
use async_trait::async_trait;

/// Core contract for a Hermes agent.
/// All five steps of the self-evolution loop, plus a default run_loop implementation.
#[async_trait]
pub trait HermesAgent: Send + Sync + 'static {
    async fn observe(&self, ctx: &Context) -> Result<Observation>;
    async fn plan(&self, obs: Observation) -> Result<Plan>;
    async fn execute(&self, plan: Plan) -> Result<ExecutionResult>;
    async fn reflect(&self, result: &ExecutionResult) -> Result<Insight>;
    async fn evolve(&mut self, insight: Insight) -> Result<()>;

    /// Default run_loop: repeatedly executes the five-step cycle
    /// until the context signals a stop.
    async fn run_loop(&mut self, ctx: Context) -> Result<()> {
        loop {
            ctx.advance_iteration();
            if ctx.should_stop() {
                tracing::info!("Stop signaled at start, exiting loop");
                break;
            }
            tracing::info!("Turn starting: observe phase");
            let obs = self.observe(&ctx).await?;
            tracing::info!("Plan phase");
            let plan = self.plan(obs).await?;
            tracing::info!(steps = plan.steps.len(), "Execute phase");
            let result = self.execute(plan).await?;
            tracing::info!(
                success = result.success,
                duration_ms = result.duration_ms,
                "Reflect phase"
            );
            let insight = self.reflect(&result).await?;
            tracing::info!(score = insight.score, strategy = %insight.strategy_id, "Evolve phase");
            self.evolve(insight).await?;
            if ctx.should_stop() {
                break;
            }
        }
        Ok(())
    }
}
