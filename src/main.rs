// src/main.rs  (~80 lines)
// Hermes Agent CLI — assembles all subsystems per the arch spec.
use std::sync::Arc;

use agent_core::AgentEvent;
use clap::Parser;
use serde::Deserialize;
use tokio::sync::mpsc::UnboundedSender;

// ── CLI ──────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "hermes", about = "Small Hermes Agent")]
struct Cli {
    #[arg(short, long, default_value = "config/default.toml")]
    config: String,
    #[arg(short, long)]
    task: Option<String>,
    /// Interactive mode: multi-turn conversation, reads tasks from stdin
    #[arg(short, long)]
    interactive: bool,
    /// Launch with terminal user interface (ratatui).
    #[arg(long)]
    tui: bool,
}

// ── Config ───────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct Config {
    learning_rate: f64,
    working_memory_size: usize,
    max_concurrency: usize,
    #[serde(default)]
    llm: LlmConfig,
    #[serde(default)]
    qdrant: QdrantConfig,
    #[serde(default)]
    search: SearchConfig,
    #[serde(default)]
    scorer: ScorerConfig,
}

impl Config {
    fn from_file(path: &str) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LlmConfig {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    max_tokens: u32,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    base_url: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5-20251001".into(),
            max_tokens: 4096,
            api_key: String::new(),
            base_url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct QdrantConfig {
    #[serde(default = "default_qdrant_url")]
    url: String,
    #[serde(default = "default_collection")]
    collection: String,
    #[serde(default = "default_embedding_dim")]
    embedding_dim: usize,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: default_qdrant_url(),
            collection: default_collection(),
            embedding_dim: default_embedding_dim(),
        }
    }
}

fn default_qdrant_url() -> String {
    "http://localhost:6334".into()
}
fn default_collection() -> String {
    "hermes_memory".into()
}
fn default_embedding_dim() -> usize {
    1024
}

#[derive(Debug, Clone, Deserialize)]
struct SearchConfig {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default = "default_search_endpoint")]
    endpoint: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            endpoint: default_search_endpoint(),
        }
    }
}

fn default_search_endpoint() -> String {
    "https://api.search.brave.com/res/v1/web/search".into()
}

#[derive(Debug, Clone, Deserialize)]
struct ScorerConfig {
    #[serde(default = "default_success_weight")]
    success_weight: f64,
    #[serde(default = "default_latency_weight")]
    latency_weight: f64,
    #[serde(default = "default_quality_weight")]
    quality_weight: f64,
    #[serde(default = "default_latency_target")]
    latency_target_ms: u64,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self {
            success_weight: default_success_weight(),
            latency_weight: default_latency_weight(),
            quality_weight: default_quality_weight(),
            latency_target_ms: default_latency_target(),
        }
    }
}

fn default_success_weight() -> f64 {
    0.6
}
fn default_latency_weight() -> f64 {
    0.2
}
fn default_quality_weight() -> f64 {
    0.2
}
fn default_latency_target() -> u64 {
    2000
}

// ── Agent struct ─────────────────────────────────────────────

struct SmallHermesAgent {
    planner: planner::Planner,
    scheduler: scheduler::Scheduler,
    reflector: reflector::Reflector,
    evolution: Arc<evolution::EvolutionEngine>,
    working_memory: memory::WorkingMemory,
    llm: Arc<dyn llm::LlmAdapter>,
    turn: u64,
    /// Optional event sender for TUI progress reporting.
    event_tx: Option<UnboundedSender<AgentEvent>>,
    /// Conversation history for multi-turn context (user_input → summary pairs).
    conversation_history: Vec<(String, String)>,
    /// TUI input state for in-terminal interactive input.
    tui_input: Option<Arc<tui::TuiInput>>,
}

#[async_trait::async_trait]
impl agent_core::agent::HermesAgent for SmallHermesAgent {
    async fn observe(
        &self,
        ctx: &agent_core::context::Context,
    ) -> anyhow::Result<agent_core::Observation> {
        let user_input = if ctx.is_interactive() {
            let task = if let Some(ref tui_input) = self.tui_input {
                // TUI mode: signal TUI to show input bar, poll for submission
                tui_input.awaiting.store(true, std::sync::atomic::Ordering::Relaxed);
                tui_input.buffer.lock().clear();
                *tui_input.submitted.lock() = None;

                let mut input = String::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if let Some(text) = tui_input.submitted.lock().take() {
                        input = text;
                        break;
                    }
                    if ctx.should_stop() {
                        break;
                    }
                }
                tui_input.awaiting.store(false, std::sync::atomic::Ordering::Relaxed);
                input
            } else {
                ctx.next_interactive_task()
            };
            if task.is_empty() || task == "exit" || task == "quit" {
                ctx.signal_stop();
                return Ok(agent_core::Observation {
                    id: uuid::Uuid::new_v4(),
                    timestamp: chrono::Utc::now(),
                    user_input: String::new(),
                    env_state: serde_json::json!({}),
                    memory_ctx: vec![],
                });
            }
            task
        } else {
            ctx.task()
                .unwrap_or("No task provided — waiting for input")
                .to_string()
        };

        let mut memory_ctx = self.working_memory.recent(5);

        // Inject conversation history as context for multi-turn coherence
        for (i, (q, a)) in self.conversation_history.iter().rev().take(5).enumerate() {
            memory_ctx.push(agent_core::MemoryChunk {
                id: uuid::Uuid::new_v4(),
                content: format!("[对话记录 #{}] 用户: {} | 结果: {}", i + 1, q, a),
                embedding: vec![],
                timestamp: chrono::Utc::now(),
            });
        }

        Ok(agent_core::Observation {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            user_input,
            env_state: serde_json::json!({
                "working_memory_size": self.working_memory.len(),
            }),
            memory_ctx,
        })
    }

    async fn plan(
        &self,
        obs: agent_core::Observation,
    ) -> anyhow::Result<agent_core::Plan> {
        self.planner.plan(obs).await
    }

    async fn execute(
        &self,
        plan: agent_core::Plan,
    ) -> anyhow::Result<agent_core::ExecutionResult> {
        let result = self.scheduler.execute(&plan).await?;

        // Display results (CLI mode only; TUI gets events from scheduler)
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

    /// Custom run loop: adds result summarization after each iteration.
    async fn run_loop(&mut self, ctx: agent_core::context::Context) -> anyhow::Result<()> {
        self.emit(AgentEvent::AgentStarted {
            name: "Hermes Agent".into(),
        });

        loop {
            self.turn += 1;
            self.emit(AgentEvent::TurnStarted { turn: self.turn });

            if ctx.is_interactive() && self.event_tx.is_none() {
                eprintln!("── 第 {} 轮 ──", self.turn);
            }

            let obs = self.observe(&ctx).await?;
            if ctx.should_stop() {
                break;
            }

            let user_input = obs.user_input.clone();

            let plan = self.plan(obs).await?;
            let result = self.execute(plan).await?;

            self.emit(AgentEvent::ReflectPhaseStarted);
            let insight = self.reflect(&result).await?;
            self.emit(AgentEvent::ReflectPhaseComplete {
                score: insight.score,
                lesson: insight.lesson.clone(),
            });

            self.emit(AgentEvent::EvolvePhaseStarted);
            self.evolve(insight).await?;
            self.emit(AgentEvent::EvolvePhaseComplete);

            // Summarize results in natural language
            let summary = self.summarize_result(&result).await;
            if let Ok(ref s) = summary {
                self.emit(AgentEvent::SummaryReady { summary: s.clone() });
                if self.event_tx.is_none() {
                    eprintln!("\n\x1b[35m📝 {}\x1b[0m\n", s);
                }
                // Record conversation history for multi-turn context
                self.conversation_history.push((user_input, s.clone()));
                // Keep last 20 exchanges to bound memory
                if self.conversation_history.len() > 20 {
                    self.conversation_history.remove(0);
                }
            }

            if ctx.should_stop() {
                break;
            }
        }
        self.emit(AgentEvent::AgentStopped);
        Ok(())
    }
}

impl SmallHermesAgent {
    /// Send an event if a sender is configured.
    fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            if tx.send(event).is_err() {
                tracing::warn!("Event channel closed — TUI observer may have disconnected");
            }
        }
    }

    /// Generate a natural-language summary of the execution for the user.
    async fn summarize_result(
        &self,
        result: &agent_core::ExecutionResult,
    ) -> anyhow::Result<String> {
        let outputs: Vec<String> = result
            .outputs
            .iter()
            .map(|o| format!("[{}] {}", if o.success { "OK" } else { "FAIL" }, o.content))
            .collect();
        let prompt = format!(
            "Summarize the following execution results in one concise Chinese sentence. \
             Focus on what was accomplished.\n\n{}",
            outputs.join("\n")
        );
        let summary = self.llm.complete(prompt).await?;
        Ok(summary.trim().to_string())
    }
}

// ── Main ─────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = Config::from_file(&cli.config)?;

    // ── Assemble dependencies (matching arch.md Section 6) ────
    let memory: Arc<dyn agent_core::MemoryStore> = Arc::new(
        memory::VectorMemory::new(&memory::VectorMemoryConfig {
            url: cfg.qdrant.url.clone(),
            collection: cfg.qdrant.collection.clone(),
            embedding_dim: cfg.qdrant.embedding_dim,
        })
        .await?,
    );

    let llm: Arc<dyn llm::LlmAdapter> = match cfg.llm.provider.as_str() {
        "openai" | "deepseek" => {
            let key = if cfg.llm.api_key.is_empty() {
                let k = std::env::var("OPENAI_API_KEY")
                    .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
                    .unwrap_or_default();
                if k.is_empty() {
                    tracing::warn!("No OpenAI/DeepSeek API key configured — API calls will fail");
                }
                k
            } else {
                cfg.llm.api_key.clone()
            };
            let base_url = if cfg.llm.base_url.is_empty() {
                if cfg.llm.provider == "deepseek" {
                    "https://api.deepseek.com/v1".to_string()
                } else {
                    "https://api.openai.com/v1".to_string()
                }
            } else {
                cfg.llm.base_url.clone()
            };
            Arc::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: key,
                model: if cfg.llm.model.is_empty() && cfg.llm.provider == "deepseek" {
                    "deepseek-chat".into()
                } else {
                    cfg.llm.model.clone()
                },
                max_tokens: cfg.llm.max_tokens,
                base_url,
            }))
        }
        _ => {
            let key = if cfg.llm.api_key.is_empty() {
                let k = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
                if k.is_empty() {
                    tracing::warn!("No Anthropic API key configured — API calls will fail");
                }
                k
            } else {
                cfg.llm.api_key.clone()
            };
            Arc::new(llm::AnthropicAdapter::new(&llm::AnthropicConfig {
                api_key: key,
                model: cfg.llm.model.clone(),
                max_tokens: cfg.llm.max_tokens,
            }))
        }
    };

    let tools = Arc::new(tools::ToolRegistry::default());
    tools.register(Arc::new(tools::BashTool));
    tools.register(Arc::new(tools::ReadFileTool));
    tools.register(Arc::new(tools::WriteFileTool));
    tools.register(Arc::new(tools::WebSearchTool::new(&tools::SearchConfig {
        api_key: cfg.search.api_key.clone(),
        endpoint: cfg.search.endpoint.clone(),
    })));

    let evolution = Arc::new(
        evolution::EvolutionEngine::load_from_file(
            ".hermes_evolution.json",
            cfg.learning_rate,
            Arc::clone(&memory),
        )
        .unwrap_or_else(|e| {
            let err_str = e.to_string();
            if err_str.contains("No such file") || err_str.contains("entity not found") {
                tracing::info!("No previous evolution state found, starting fresh");
            } else {
                tracing::warn!(error = %e, "Failed to load evolution state, starting fresh");
            }
            evolution::EvolutionEngine::new(cfg.learning_rate, Arc::clone(&memory))
        }),
    );

    let mut planner = planner::Planner::new(
        Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        Arc::clone(&evolution),
    )
    .with_streaming(true);
    planner.set_tools(tools.describe_all());

    let mut scheduler = scheduler::Scheduler::new(Arc::clone(&tools), cfg.max_concurrency);

    let reflector = if cfg.scorer.success_weight != 0.6
        || cfg.scorer.latency_weight != 0.2
    {
        // Custom scorer config
        reflector::Reflector::with_scorer(
            Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
            evolution::Scorer {
                success_weight: cfg.scorer.success_weight,
                latency_weight: cfg.scorer.latency_weight,
                quality_weight: cfg.scorer.quality_weight,
                latency_target_ms: cfg.scorer.latency_target_ms,
            },
        )
    } else {
        // Default scorer
        reflector::Reflector::new(Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>)
    };

    let evolution_handle = Arc::clone(&evolution);

    // ── Create event channel (TUI mode only) ────────────────
    let (event_tx, event_rx) = if cli.tui {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        planner.set_event_sender(tx.clone());
        scheduler.set_event_sender(tx.clone());
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let tui_input = if cli.tui {
        Some(tui::TuiInput::new())
    } else {
        None
    };

    let agent = SmallHermesAgent {
        planner,
        scheduler,
        reflector,
        evolution,
        working_memory: memory::WorkingMemory::new(cfg.working_memory_size),
        llm: Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        turn: 0,
        event_tx,
        conversation_history: Vec::new(),
        tui_input: tui_input.clone(),
    };

    tracing::info!(
        "Hermes agent starting: provider={}, model={}, config={}, interactive={}, tui={}",
        cfg.llm.provider,
        cfg.llm.model,
        cli.config,
        cli.interactive,
        cli.tui,
    );

    // ── Run ──────────────────────────────────────────────────
    if cli.tui {
        let ctx = if cli.interactive {
            agent_core::context::Context::interactive()
        } else {
            agent_core::context::Context::new(cli.task)
        };
        tui::run_tui(
            agent,
            ctx,
            event_rx.unwrap(),
            Arc::clone(&evolution_handle),
            "Hermes Agent".into(),
            tui_input.unwrap(),
        )
        .await?;
    } else if cli.task.is_some() || cli.interactive {
        let ctx = if cli.interactive {
            eprintln!(
                "\n\x1b[1;35m🜁 Hermes Agent — 交互模式\x1b[0m\n\
                 \x1b[2m输入任务开始对话，输入 exit 或 Ctrl+C 退出\x1b[0m\n"
            );
            agent_core::context::Context::interactive()
        } else {
            agent_core::context::Context::new(cli.task)
        };
        agent_core::runner::run_agent(agent, ctx).await?;
    } else {
        // Continuous loop mode (no task, not interactive)
        let ctx = agent_core::context::Context::new(None);
        agent_core::runner::run_agent(agent, ctx).await?;
    }

    // Save evolution state on exit
    if let Err(e) = evolution_handle.save_to_file(".hermes_evolution.json") {
        tracing::warn!(error = %e, "Failed to save evolution state");
        anyhow::bail!("Failed to save evolution state: {e}");
    }

    tracing::info!("Hermes Agent stopped.");
    Ok(())
}
