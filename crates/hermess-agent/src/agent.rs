// crates/hermess-agent/src/agent.rs
use agent_core::context::Context;
use agent_core::{AgentEvent, ExecutionResult, HermesAgent, Insight, MemoryChunk, Observation, Plan};
use async_trait::async_trait;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// The default agent implementation, shared by CLI and web entrypoints.
pub struct SmallHermesAgent {
    pub planner: planner::Planner,
    pub scheduler: scheduler::Scheduler,
    pub reflector: reflector::Reflector,
    pub evolution: Arc<evolution::EvolutionEngine>,
    pub working_memory: memory::WorkingMemory,
    pub llm: Arc<dyn llm::LlmAdapter>,
    pub turn: u64,
    /// Token usage tracker for cost estimation.
    pub usage_tracker: Arc<llm::UsageTracker>,
    /// Optional event sender for TUI / web progress reporting.
    pub event_tx: Option<UnboundedSender<AgentEvent>>,
    /// Max replanning attempts when all execution steps fail.
    pub max_replans: usize,
    /// Threshold: compress history when it exceeds this many entries.
    pub compress_threshold: usize,
    /// Fraction of history to keep after compression (0.0–1.0).
    pub compress_keep_ratio: f64,
    /// Conversation history for multi-turn context (user_input → summary pairs).
    pub conversation_history: Vec<(String, String)>,
    /// TUI input state for in-terminal interactive input.
    #[cfg(feature = "tui")]
    pub tui_input: Option<Arc<tui::TuiInput>>,
}

impl SmallHermesAgent {
    /// 记录最近一次 LLM 调用的 token 用量。
    fn record_usage(&self) {
        if let Some(u) = self.llm.last_usage() {
            self.usage_tracker.record(&u);
        }
    }

    /// Send an event if a sender is configured. Falls back to stderr in CLI mode.
    pub fn emit(&self, event: AgentEvent) {
        // CLI 模式下直接打印关键事件到 stderr
        if self.event_tx.is_none() {
            match &event {
                AgentEvent::AgentError { message } => {
                    eprintln!("\n\x1b[31m✗ 错误: {}\x1b[0m", message);
                }
                AgentEvent::SummaryReady { summary } => {
                    eprintln!("\n\x1b[35m📝 {}\x1b[0m\n", summary);
                }
                _ => {}
            }
        }
        if let Some(ref tx) = self.event_tx {
            if tx.send(event).is_err() {
                tracing::warn!("Event channel closed — observer may have disconnected");
            }
        }
    }

    /// Generate a natural-language summary, streaming tokens to the TUI when available.
    /// If all steps used `reply`, returns the reply content directly without re-summarizing.
    pub async fn summarize_result(
        &self,
        result: &ExecutionResult,
    ) -> anyhow::Result<String> {
        // 如果所有成功的步骤都是 reply，直接返回内容（无需 LLM 重述）
        let reply_outputs: Vec<&str> = result
            .outputs
            .iter()
            .filter(|o| o.tool == "reply" && o.success)
            .map(|o| o.content.as_str())
            .collect();
        if !reply_outputs.is_empty() && result.outputs.iter().all(|o| o.tool == "reply") {
            return Ok(reply_outputs.join("\n"));
        }

        let outputs: Vec<String> = result
            .outputs
            .iter()
            .map(|o| format!("[{}:{}] {}", o.tool, if o.success { "OK" } else { "FAIL" }, o.content))
            .collect();
        let prompt = format!(
            "Summarize the following execution results in one concise Chinese sentence. \
             Focus on what was accomplished.\n\n{}",
            outputs.join("\n")
        );

        // Stream tokens to TUI when an event sender is configured
        if let Some(ref tx) = self.event_tx {
            let mut stream = self.llm.complete_stream(prompt).await?;
            let mut full = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(token) => {
                        let _ = tx.send(AgentEvent::SummaryStreamingToken {
                            token: token.clone(),
                        });
                        full.push_str(&token);
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(full.trim().to_string())
        } else {
            let summary = self.llm.complete(prompt).await?;
            Ok(summary.trim().to_string())
        }
    }

    /// 保存会话状态到文件，用于 `--save`。
    pub fn save_state(&self, path: &str) -> anyhow::Result<()> {
        let chunks: Vec<agent_core::MemoryChunk> = self.working_memory.all();
        let state = agent_core::SessionState::new(
            self.turn,
            self.conversation_history.clone(),
            chunks,
        );
        state.save_to_file(path)
    }

    /// 从文件恢复会话状态，用于 `--resume`。
    pub fn restore_state(&mut self, path: &str) -> anyhow::Result<()> {
        let state = agent_core::SessionState::load_from_file(path)?;
        self.turn = state.turn;
        self.conversation_history = state.conversation_history;
        for chunk in state.working_memory_chunks {
            self.working_memory.push(chunk);
        }
        tracing::info!(
            turn = self.turn,
            history_entries = self.conversation_history.len(),
            "会话状态已恢复"
        );
        Ok(())
    }

    /// 当对话历史超过阈值时，用 LLM 压缩最旧的一半条目为一条摘要。
    pub async fn compress_history(&mut self) {
        if self.conversation_history.len() <= self.compress_threshold {
            return;
        }

        let keep = (self.conversation_history.len() as f64 * self.compress_keep_ratio) as usize;
        let to_compress: Vec<String> = self
            .conversation_history
            .iter()
            .take(self.conversation_history.len().saturating_sub(keep))
            .map(|(q, a)| format!("用户: {} | 结果: {}", q, a))
            .collect();

        if to_compress.is_empty() {
            return;
        }

        let prompt = format!(
            "Compress the following conversation history into a single concise summary. \
             Preserve key context, decisions, and outcomes.\n\n{}",
            to_compress.join("\n")
        );

        match self.llm.complete(prompt).await {
            Ok(compressed) => {
                let mut new_history: Vec<(String, String)> = vec![(
                    "[历史摘要]".to_string(),
                    compressed.trim().to_string(),
                )];
                let keep_start = self.conversation_history.len().saturating_sub(keep);
                let rest = self.conversation_history.split_off(keep_start);
                new_history.extend(rest);
                self.conversation_history = new_history;
                self.record_usage();
                tracing::info!(
                    compressed_entries = to_compress.len(),
                    remaining = self.conversation_history.len(),
                    "上下文已压缩"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "上下文压缩失败，保留原始历史");
            }
        }
    }

}

/// Build a simple fallback summary from raw step outputs when LLM summarization fails.
fn build_fallback_summary(result: &ExecutionResult) -> String {
    let success_count = result.outputs.iter().filter(|o| o.success).count();
    let total = result.outputs.len();
    let previews: Vec<String> = result
        .outputs
        .iter()
        .take(3)
        .map(|o| {
            let status = if o.success { "OK" } else { "FAIL" };
            let preview = if o.content.len() > 200 {
                format!("{}...", &o.content[..200])
            } else {
                o.content.clone()
            };
            format!("[{}] {}", status, preview)
        })
        .collect();
    format!(
        "执行完成: {}/{} 步骤成功。\n{}",
        success_count,
        total,
        previews.join("\n")
    )
}

#[async_trait]
impl HermesAgent for SmallHermesAgent {
    async fn observe(
        &self,
        ctx: &Context,
    ) -> anyhow::Result<Observation> {
        let user_input = if ctx.is_interactive() {
            #[cfg(feature = "tui")]
            {
                if let Some(ref tui_input) = self.tui_input {
                    tui_input
                        .awaiting
                        .store(true, std::sync::atomic::Ordering::Relaxed);
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
                    tui_input
                        .awaiting
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                    input
                } else {
                    ctx.next_interactive_task()
                }
            }
            #[cfg(not(feature = "tui"))]
            {
                ctx.next_interactive_task()
            }
        } else {
            ctx.task()
                .unwrap_or("No task provided — waiting for input")
                .to_string()
        };

        if user_input.is_empty() || user_input == "exit" || user_input == "quit" {
            ctx.signal_stop();
            return Ok(Observation {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                user_input: String::new(),
                env_state: serde_json::json!({}),
                memory_ctx: vec![],
            });
        }

        let mut memory_ctx = self.working_memory.recent(5);

        // Inject conversation history as context for multi-turn coherence
        for (i, (q, a)) in self.conversation_history.iter().rev().take(5).enumerate() {
            memory_ctx.push(MemoryChunk {
                id: uuid::Uuid::new_v4(),
                content: format!("[对话记录 #{}] 用户: {} | 结果: {}", i + 1, q, a),
                embedding: vec![],
                timestamp: chrono::Utc::now(),
            });
        }

        Ok(Observation {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            user_input,
            env_state: serde_json::json!({
                "working_memory_size": self.working_memory.len(),
            }),
            memory_ctx,
        })
    }

    async fn plan(&self, obs: Observation) -> anyhow::Result<Plan> {
        self.planner.plan(obs).await
    }

    async fn execute(&self, plan: Plan) -> anyhow::Result<ExecutionResult> {
        let result = self.scheduler.execute(&plan).await?;

        for output in &result.outputs {
            self.working_memory.push(MemoryChunk {
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
        result: &ExecutionResult,
    ) -> anyhow::Result<Insight> {
        self.reflector.reflect(result).await
    }

    async fn evolve(&mut self, insight: Insight) -> anyhow::Result<()> {
        self.evolution.update(insight).await
    }

    /// Custom run loop: adds result summarization after each iteration.
    async fn run_loop(&mut self, ctx: Context) -> anyhow::Result<()> {
        self.emit(AgentEvent::AgentStarted {
            name: "Hermes Agent".into(),
        });

        loop {
            self.turn += 1;
            self.emit(AgentEvent::TurnStarted { turn: self.turn });

            ctx.advance_iteration();
            if ctx.should_stop() {
                break;
            }

            if ctx.is_interactive() && self.event_tx.is_none() {
                eprintln!("── 第 {} 轮 ──", self.turn);
            }

            let obs = self.observe(&ctx).await?;
            if ctx.should_stop() {
                break;
            }

            let user_input = obs.user_input.clone();

            // Clone obs for potential replanning (plan() consumes the original)
            let obs_for_replan = obs.clone();
            let mut plan = match self.plan(obs).await {
                Ok(p) => {
                    self.record_usage();
                    p
                }
                Err(e) => {
                    let msg = format!("Plan 阶段失败: {:#}", e);
                    self.emit(AgentEvent::AgentError {
                        message: msg.clone(),
                    });
                    self.emit(AgentEvent::SummaryReady { summary: msg });
                    break;
                }
            };

            // Execute with replan support: when execution has failures, the LLM
            // proposes alternative approaches up to max_replans times.
            let mut result = match self.execute(plan).await {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("Execute 阶段失败: {:#}", e);
                    self.emit(AgentEvent::AgentError {
                        message: msg.clone(),
                    });
                    self.emit(AgentEvent::SummaryReady { summary: msg });
                    break;
                }
            };

            let mut replan_attempt = 0;
            while !result.success && replan_attempt < self.max_replans {
                replan_attempt += 1;
                let failed_count = result.outputs.iter().filter(|o| !o.success).count();
                self.emit(AgentEvent::ReplanNeeded {
                    reason: format!("{} 个步骤失败，尝试替代方案", failed_count),
                    attempt: replan_attempt,
                });

                match self.planner.replan(&obs_for_replan, &result).await {
                    Ok(new_plan) => {
                        self.record_usage();
                        self.emit(AgentEvent::ReplanComplete {
                            new_steps_count: new_plan.steps.len(),
                        });
                        plan = new_plan;
                    }
                    Err(e) => {
                        let msg = format!("Replan 阶段失败: {:#}", e);
                        self.emit(AgentEvent::AgentError { message: msg });
                        break;
                    }
                }

                match self.execute(plan).await {
                    Ok(r) => result = r,
                    Err(e) => {
                        let msg = format!("Execute 阶段失败: {:#}", e);
                        self.emit(AgentEvent::AgentError {
                            message: msg.clone(),
                        });
                        self.emit(AgentEvent::SummaryReady { summary: msg });
                        break;
                    }
                }
            }

            self.emit(AgentEvent::ReflectPhaseStarted);
            let insight = match self.reflect(&result).await {
                Ok(i) => {
                    self.record_usage();
                    i
                }
                Err(e) => {
                    let msg = format!("Reflect 阶段失败: {:#}", e);
                    self.emit(AgentEvent::AgentError {
                        message: msg.clone(),
                    });
                    self.emit(AgentEvent::SummaryReady { summary: msg });
                    break;
                }
            };
            self.emit(AgentEvent::ReflectPhaseComplete {
                score: insight.score,
                lesson: insight.lesson.clone(),
            });

            self.emit(AgentEvent::EvolvePhaseStarted);
            if let Err(e) = self.evolve(insight).await {
                let msg = format!("Evolve 阶段失败: {:#}", e);
                self.emit(AgentEvent::AgentError { message: msg });
                // evolve 失败不终止，继续总结
            }
            self.emit(AgentEvent::EvolvePhaseComplete);

            let summary = self.summarize_result(&result).await;
            self.record_usage();
            match summary {
                Ok(ref s) => {
                    self.emit(AgentEvent::SummaryReady { summary: s.clone() });
                    self.conversation_history.push((user_input, s.clone()));
                    self.compress_history().await;
                }
                Err(e) => {
                    // 总结失败，回退到原始执行结果
                    let fallback = build_fallback_summary(&result);
                    let msg = format!("{}. (总结失败: {:#})", fallback, e);
                    self.emit(AgentEvent::AgentError {
                        message: format!("Summarize 失败: {:#}", e),
                    });
                    self.emit(AgentEvent::SummaryReady { summary: msg });
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
