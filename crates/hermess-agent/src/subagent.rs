// crates/hermess-agent/src/subagent.rs
// Sub-agent executor: spawns an independent SmallHermesAgent for each
// delegable step. The sub-agent runs a single observe→plan→execute→reflect
// pipeline, then returns the result to the coordinator.

use std::sync::Arc;
use std::time::Instant;

use agent_core::{AgentEvent, HermesAgent};
use llm::LlmAdapter;
use scheduler::{SubAgentOutput, SubAgentRunner};
use tokio::sync::mpsc::UnboundedSender;

/// Result returned by a sub-agent after completing its delegated task.
#[derive(Debug, Clone)]
pub struct SubAgentResult {
    pub summary: String,
    pub success: bool,
    pub duration_ms: u64,
    pub tool_outputs: Vec<agent_core::StepOutput>,
}

/// Factory-owned handle that implements [`SubAgentRunner`] so the scheduler
/// can spawn sub-agents without depending on the agent crate directly.
pub struct SubAgentRunnerImpl {
    llm: Arc<dyn LlmAdapter>,
    evolution: Arc<evolution::EvolutionEngine>,
    tools: Arc<tools::ToolRegistry>,
    event_tx: Option<UnboundedSender<AgentEvent>>,
}

impl SubAgentRunnerImpl {
    pub fn new(
        llm: Arc<dyn LlmAdapter>,
        evolution: Arc<evolution::EvolutionEngine>,
        tools: Arc<tools::ToolRegistry>,
        event_tx: Option<UnboundedSender<AgentEvent>>,
    ) -> Self {
        Self {
            llm,
            evolution,
            tools,
            event_tx,
        }
    }
}

#[async_trait::async_trait]
impl SubAgentRunner for SubAgentRunnerImpl {
    async fn run(&self, task: &str) -> anyhow::Result<SubAgentOutput> {
        let result = run_subagent(
            task,
            Arc::clone(&self.llm) as Arc<dyn LlmAdapter>,
            Arc::clone(&self.evolution),
            Arc::clone(&self.tools),
            self.event_tx.clone(),
        )
        .await?;
        Ok(SubAgentOutput {
            summary: result.summary,
            success: result.success,
            duration_ms: result.duration_ms,
        })
    }
}

/// Spawn a sub-agent to autonomously handle a single delegable step.
///
/// The sub-agent gets its own Planner, Scheduler, Reflector, and WorkingMemory,
/// but shares the coordinator's LLM connection, EvolutionEngine, and ToolRegistry.
/// It runs exactly one iteration of the observe→plan→execute→reflect pipeline.
pub async fn run_subagent(
    task: &str,
    coordinator_llm: Arc<dyn LlmAdapter>,
    evolution: Arc<evolution::EvolutionEngine>,
    tools: Arc<tools::ToolRegistry>,
    event_tx: Option<UnboundedSender<AgentEvent>>,
) -> anyhow::Result<SubAgentResult> {
    let start = Instant::now();

    // Build a minimal planner for the sub-agent. No streaming to avoid
    // mixing output with the coordinator's TUI/CLI display.
    let mut planner = planner::Planner::new(
        Arc::clone(&coordinator_llm) as Arc<dyn LlmAdapter>,
        Arc::clone(&evolution),
    );
    planner.set_tools(tools.describe_all());

    // Build a scheduler that does NOT support further delegation,
    // preventing unbounded recursion.
    let scheduler = scheduler::Scheduler::new(Arc::clone(&tools), 4);

    let reflector = reflector::Reflector::new(Arc::clone(&coordinator_llm) as Arc<dyn LlmAdapter>);

    let mut sub_agent = crate::agent::SmallHermesAgent {
        planner,
        scheduler,
        reflector,
        evolution: Arc::clone(&evolution),
        working_memory: memory::WorkingMemory::new(10),
        llm: Arc::clone(&coordinator_llm) as Arc<dyn LlmAdapter>,
        turn: 0,
        usage_tracker: Arc::new(llm::UsageTracker::new("subagent")),
        event_tx: event_tx.clone(),
        max_replans: 0,          // no replanning for sub-agents
        compress_threshold: 100, // effectively disable compression
        compress_keep_ratio: 1.0,
        conversation_history: Vec::new(),
        recent_insights: Vec::new(),
        distiller: crate::distiller::SkillDistiller::new(),
        #[cfg(feature = "tui")]
        tui_input: None,
    };

    // Emit start event
    if let Some(ref tx) = event_tx {
        let _ = tx.send(AgentEvent::SubAgentStarted {
            task: task.to_string(),
        });
    }

    let ctx = agent_core::context::Context::new(Some(task.to_string()));

    // Run the sub-agent pipeline. We call run_loop directly but it will
    // run exactly one iteration because Context::new(Some(_)) sets max_iterations=1.
    let run_result = sub_agent.run_loop(ctx).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match run_result {
        Ok(()) => {
            // Collect the last conversation entry as the summary
            let summary = sub_agent
                .conversation_history
                .last()
                .map(|(_, a)| a.clone())
                .unwrap_or_else(|| format!("子任务完成: {task}"));

            if let Some(ref tx) = event_tx {
                let _ = tx.send(AgentEvent::SubAgentCompleted {
                    task: task.to_string(),
                    summary: summary.clone(),
                });
            }

            Ok(SubAgentResult {
                summary,
                success: true,
                duration_ms,
                tool_outputs: vec![],
            })
        }
        Err(e) => {
            if let Some(ref tx) = event_tx {
                let _ = tx.send(AgentEvent::SubAgentCompleted {
                    task: task.to_string(),
                    summary: format!("失败: {e:#}"),
                });
            }

            Ok(SubAgentResult {
                summary: format!("子任务执行失败: {e:#}"),
                success: false,
                duration_ms,
                tool_outputs: vec![],
            })
        }
    }
}
