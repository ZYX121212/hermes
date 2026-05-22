// crates/agent-core/src/agent.rs
use async_trait::async_trait;
use anyhow::Result;
use crate::{ExecutionResult, Insight, Observation, Plan};
use crate::context::Context;

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
            let obs = self.observe(&ctx).await?;
            let plan = self.plan(obs).await?;
            let result = self.execute(plan).await?;
            let insight = self.reflect(&result).await?;
            self.evolve(insight).await?;
            if ctx.should_stop() {
                break;
            }
        }
        Ok(())
    }
}
