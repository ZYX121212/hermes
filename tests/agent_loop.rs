// tests/integration/agent_loop.rs
// Integration tests for the full five-step agent loop.
use std::sync::Arc;

use async_trait::async_trait;
use agent_core::agent::HermesAgent;
use agent_core::context::Context;
use evolution::{EvolutionEngine, Scorer};
use llm::LlmAdapter;
use memory::WorkingMemory;
use memory::MockMemoryStore;
use planner::Planner;
use reflector::Reflector;
use scheduler::Scheduler;
use tools::ToolRegistry;

/// Mock LLM adapter that returns predetermined responses for testing.
struct MockLlm {
    plan_response: String,
    embed_response: Vec<f32>,
}

impl MockLlm {
    fn new(plan_response: &str) -> Self {
        Self {
            plan_response: plan_response.to_string(),
            embed_response: vec![0.1_f32; 1024],
        }
    }
}

#[async_trait]
impl LlmAdapter for MockLlm {
    async fn complete(&self, _prompt: String) -> anyhow::Result<String> {
        Ok(self.plan_response.clone())
    }

    async fn complete_stream(
        &self,
        _prompt: String,
    ) -> anyhow::Result<Box<dyn futures::Stream<Item = anyhow::Result<String>> + Unpin + Send>>
    {
        let chunks: Vec<anyhow::Result<String>> = self
            .plan_response
            .split_whitespace()
            .map(|w| Ok(format!("{w} ")))
            .collect();
        Ok(Box::new(futures::stream::iter(chunks)))
    }

    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(self.embed_response.clone())
    }
}

/// Minimal agent for integration testing.
struct TestAgent {
    planner: Planner,
    scheduler: Scheduler,
    reflector: Reflector,
    evolution: Arc<EvolutionEngine>,
    working_memory: WorkingMemory,
}

#[async_trait]
impl HermesAgent for TestAgent {
    async fn observe(
        &self,
        ctx: &agent_core::context::Context,
    ) -> anyhow::Result<agent_core::Observation> {
        Ok(agent_core::Observation {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            user_input: ctx.task().unwrap_or("test task").to_string(),
            env_state: serde_json::json!({}),
            memory_ctx: self.working_memory.recent(5),
        })
    }

    async fn plan(&self, obs: agent_core::Observation) -> anyhow::Result<agent_core::Plan> {
        self.planner.plan(obs).await
    }

    async fn execute(
        &self,
        plan: agent_core::Plan,
    ) -> anyhow::Result<agent_core::ExecutionResult> {
        let result = self.scheduler.execute(&plan).await?;
        // Store step outputs in working memory (mirrors SmallHermesAgent behavior)
        for output in &result.outputs {
            self.working_memory.push(agent_core::MemoryChunk {
                id: uuid::Uuid::new_v4(),
                content: format!(
                    "step={} success={} content={}",
                    output.step_id, output.success, output.content
                ),
                embedding: vec![],
                timestamp: chrono::Utc::now(),
            });
        }
        Ok(result)
    }

    async fn reflect(
        &self,
        result: &agent_core::ExecutionResult,
    ) -> anyhow::Result<agent_core::Insight> {
        self.reflector.reflect(result).await
    }

    async fn evolve(&mut self, insight: agent_core::Insight) -> anyhow::Result<()> {
        self.evolution.update(insight).await
    }
}

fn build_test_agent(plan_response: &str) -> TestAgent {
    let llm: Arc<dyn LlmAdapter> = Arc::new(MockLlm::new(plan_response));
    let memory: Arc<dyn agent_core::MemoryStore> = Arc::new(MockMemoryStore::new());
    let evolution = Arc::new(EvolutionEngine::new(0.1, memory));
    let mut planner = Planner::new(Arc::clone(&llm), Arc::clone(&evolution));

    let tool_registry = Arc::new(ToolRegistry::default());
    tool_registry.register(Arc::new(tools::BashTool));
    planner.set_tools(tool_registry.describe_all());

    let scheduler = Scheduler::new(Arc::clone(&tool_registry), 4);
    let scorer = Scorer::default();
    let reflector = Reflector::with_scorer(Arc::clone(&llm), scorer);
    let working_memory = WorkingMemory::new(50);

    TestAgent {
        planner,
        scheduler,
        reflector,
        evolution,
        working_memory,
    }
}

#[tokio::test]
async fn test_single_step_bash_plan() {
    let plan_json = r#"[
        {
            "tool": "bash",
            "args": {"command": "echo hello"},
            "depends": [],
            "candidates": ["fast", "safe"]
        }
    ]"#;

    let agent = build_test_agent(plan_json);
    let ctx = Context::new(Some("say hello".into()));

    // Run one iteration of the loop manually (not run_loop, which loops forever)
    let obs = agent.observe(&ctx).await.unwrap();
    let plan = agent.plan(obs).await.unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].tool, "bash");
}

#[tokio::test]
async fn test_multi_step_plan_with_dependencies() {
    let plan_json = r#"[
        {
            "tool": "bash",
            "args": {"command": "echo step1"},
            "depends": [],
            "candidates": ["fast"]
        },
        {
            "tool": "bash",
            "args": {"command": "echo step2"},
            "depends": [0],
            "candidates": ["fast"]
        },
        {
            "tool": "web_search",
            "args": {"query": "test"},
            "depends": [],
            "candidates": ["thorough"]
        }
    ]"#;

    let agent = build_test_agent(plan_json);
    let ctx = Context::new(Some("multi-step task".into()));

    let obs = agent.observe(&ctx).await.unwrap();
    let plan = agent.plan(obs).await.unwrap();
    assert_eq!(plan.steps.len(), 3);

    // Step 0 and step 2 should be in the first layer (no dependencies)
    let layers = plan.dag.topological_layers(&plan.steps);
    assert!(!layers.is_empty());
    // Two independent steps in layer 0
    assert_eq!(layers[0].len(), 2);
    // One step in layer 1 (depends on step 0)
    assert_eq!(layers[1].len(), 1);
}

#[tokio::test]
async fn test_full_loop_single_iteration() {
    let plan_json = r#"[
        {
            "tool": "bash",
            "args": {"command": "echo integration_test"},
            "depends": [],
            "candidates": ["fast"]
        }
    ]"#;

    let mut agent = build_test_agent(plan_json);
    let ctx = Context::new(Some("integration test".into()));

    // Manual single iteration of the five-step loop
    let obs = agent.observe(&ctx).await.unwrap();
    let plan = agent.plan(obs).await.unwrap();
    let result = agent.execute(plan).await.unwrap();
    let insight = agent.reflect(&result).await.unwrap();
    agent.evolve(insight).await.unwrap();

    // After one iteration, the evolution engine should have learned something
    assert!(agent.evolution.strategy_count() > 0, "Evolution engine should have registered at least one strategy");
    let weights = agent.evolution.all_weights();
    assert!(!weights.is_empty(), "Should have at least one strategy weight");

    // Working memory should have an entry from the execution
    assert!(agent.working_memory.len() > 0);
}

#[tokio::test]
async fn test_execution_result_reflects_partial_failure() {
    let plan_json = r#"[
        {
            "tool": "bash",
            "args": {"command": "false"},
            "depends": [],
            "candidates": ["risky"]
        }
    ]"#;

    let agent = build_test_agent(plan_json);
    let ctx = Context::new(Some("failing command".into()));

    let obs = agent.observe(&ctx).await.unwrap();
    let plan = agent.plan(obs).await.unwrap();
    let result = agent.execute(plan).await.unwrap();

    // The bash command "false" exits with code 1
    assert!(!result.success, "Expected execution to fail with 'false' command");

    let insight = agent.reflect(&result).await.unwrap();
    // Score should be negative for a failed execution
    assert!(insight.score < 0.0, "Expected negative score for failure, got {}", insight.score);
}
