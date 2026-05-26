// crates/planner/src/planner.rs
// LLM-driven task planner: decomposes a user goal into executable steps
// arranged in a DAG, with optimal strategies selected from the evolution engine.
use std::sync::Arc;

use agent_core::{AgentEvent, Observation, Plan, Step};
use anyhow::Result;
use evolution::EvolutionEngine;
use futures::StreamExt;
use llm::LlmAdapter;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::dependency::build_dag;
use crate::plan::StepSpec;

/// Task planner that uses an LLM to decompose a goal into executable steps,
/// then assigns optimal strategies from the evolution engine.
pub struct Planner {
    llm: Arc<dyn LlmAdapter>,
    evolution: Arc<EvolutionEngine>,
    /// Available tools and their schemas for prompt injection.
    tool_descriptions: Vec<serde_json::Value>,
    /// Whether to stream LLM output to stderr in real-time.
    streaming: bool,
    /// Optional event sender for TUI/observer integration.
    event_tx: Option<UnboundedSender<AgentEvent>>,
}

impl Planner {
    pub fn new(llm: Arc<dyn LlmAdapter>, evolution: Arc<EvolutionEngine>) -> Self {
        Self {
            llm,
            evolution,
            tool_descriptions: vec![],
            streaming: false,
            event_tx: None,
        }
    }

    /// Set an event sender for TUI progress reporting.
    pub fn set_event_sender(&mut self, tx: UnboundedSender<AgentEvent>) {
        self.event_tx = Some(tx);
    }

    /// Enable streaming output of LLM responses.
    pub fn with_streaming(mut self, enabled: bool) -> Self {
        self.streaming = enabled;
        self
    }

    /// Set the available tool descriptions for prompt construction.
    pub fn set_tools(&mut self, tools: Vec<serde_json::Value>) {
        self.tool_descriptions = tools;
    }

    /// Decompose an observation into a plan with steps and dependencies.
    pub async fn plan(&self, obs: Observation) -> Result<Plan> {
        self.emit(AgentEvent::PlanPhaseStarted);

        let prompt = self.build_prompt(&obs);
        let raw = if self.streaming {
            self.complete_streaming(prompt).await?
        } else {
            self.llm.complete(prompt).await?
        };
        let raw = Self::extract_json(&raw);
        let (specs, _retried) = match serde_json::from_str::<Vec<StepSpec>>(raw) {
            Ok(specs) if !specs.is_empty() => (specs, false),
            _ => {
                self.emit(AgentEvent::PlanRetry);
                tracing::warn!("Plan parse failed, retrying with clarification prompt...");
                // Retry once with a clarification prompt
                let retry_prompt = format!(
                    "Your previous response was invalid or empty. You MUST return a valid JSON array \
                     of steps. Each step object requires: \"tool\" (string), \"args\" (object), \
                     \"depends\" (number array), \"candidates\" (string array).\n\n\
                     Original task: {}\n\nReturn ONLY the JSON array:",
                    obs.user_input
                );
                let retry_raw = if self.streaming {
                    eprint!("  \x1b[33mretry\x1b[0m ");
                    self.complete_streaming(retry_prompt).await?
                } else {
                    self.llm.complete(retry_prompt).await?
                };
                let retry_raw = Self::extract_json(&retry_raw);
                let specs: Vec<StepSpec> = serde_json::from_str(retry_raw).map_err(|e| {
                    anyhow::anyhow!("计划解析失败，请重新描述你的任务。\n错误: {e}")
                })?;
                if specs.is_empty() {
                    return Err(anyhow::anyhow!(
                        "无法为此任务生成执行计划，请尝试更具体的描述。"
                    ));
                }
                (specs, true)
            }
        };

        // Assign UUIDs upfront (needed for dependency resolution)
        let step_ids: Vec<Uuid> = specs.iter().map(|_| Uuid::new_v4()).collect();

        // Build dependency DAG from specs
        let dag = build_dag(&specs, &step_ids)?;

        // Resolve each step with the best strategy
        let steps: Vec<Step> = specs
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let candidates: Vec<&str> = s.candidates.iter().map(|c| c.as_str()).collect();
                let strategy = self
                    .evolution
                    .best_strategy(&candidates)
                    .unwrap_or_else(|| {
                        if !candidates.is_empty() {
                            tracing::info!(
                                tool = %s.tool,
                                candidates = ?candidates,
                                "no strategy data available, using default"
                            );
                        }
                        "default".into()
                    });

                let depends: Vec<Uuid> = s.depends.iter().map(|&d| step_ids[d]).collect();

                Step {
                    id: step_ids[i],
                    tool: s.tool,
                    args: s.args,
                    depends,
                    strategy,
                    tool_candidates: s.tool_candidates,
                    delegable: s.delegable,
                }
            })
            .collect();

        tracing::info!(
            "Planned {} steps across the DAG for task: {}",
            steps.len(),
            obs.user_input
        );

        self.emit(AgentEvent::PlanReady {
            steps_count: steps.len(),
        });

        Ok(Plan {
            id: Uuid::new_v4(),
            steps,
            dag,
        })
    }

    /// Replan after execution failure. Takes the original observation and the failed
    /// execution result, asks the LLM to propose an alternative approach.
    pub async fn replan(&self, obs: &Observation, failed: &agent_core::ExecutionResult) -> Result<Plan> {
        self.emit(AgentEvent::PlanPhaseStarted);

        let failures: Vec<String> = failed
            .outputs
            .iter()
            .filter(|o| !o.success)
            .map(|o| format!("- step {} (tool={}): {}", o.step_id, o.tool, o.content))
            .collect();

        let prompt = format!(
            "The previous plan failed. Here are the failed steps:\n{}\n\n\
             Original task: {}\n\n\
             Propose an ALTERNATIVE plan using different tools or approaches. \
             Avoid the tools that already failed. \
             Return ONLY a JSON array of steps with the same format as before.\n\n\
             Available tools:\n{}\n",
            failures.join("\n"),
            obs.user_input,
            serde_json::to_string_pretty(&self.tool_descriptions).unwrap_or_default(),
        );

        let raw = if self.streaming {
            self.complete_streaming(prompt).await?
        } else {
            self.llm.complete(prompt).await?
        };
        let raw = Self::extract_json(&raw);
        let specs: Vec<StepSpec> = serde_json::from_str(raw).map_err(|e| {
            anyhow::anyhow!("重规划解析失败: {e}")
        })?;
        if specs.is_empty() {
            return Err(anyhow::anyhow!("重规划未能生成有效步骤"));
        }

        let step_ids: Vec<Uuid> = specs.iter().map(|_| Uuid::new_v4()).collect();
        let dag = build_dag(&specs, &step_ids)?;

        let steps: Vec<Step> = specs
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let candidates: Vec<&str> = s.candidates.iter().map(|c| c.as_str()).collect();
                let strategy = self.evolution.best_strategy(&candidates).unwrap_or("default".into());
                let depends: Vec<Uuid> = s.depends.iter().map(|&d| step_ids[d]).collect();
                Step {
                    id: step_ids[i],
                    tool: s.tool,
                    args: s.args,
                    depends,
                    strategy,
                    tool_candidates: s.tool_candidates,
                    delegable: s.delegable,
                }
            })
            .collect();

        tracing::info!("Replanned {} steps", steps.len());
        self.emit(AgentEvent::ReplanComplete {
            new_steps_count: steps.len(),
        });

        Ok(Plan { id: Uuid::new_v4(), steps, dag })
    }

    /// Send an event if a sender is configured.
    fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            if tx.send(event).is_err() {
                tracing::warn!("Planner event channel closed");
            }
        }
    }

    /// Build a planning prompt that includes user goal, available tools,
    /// and relevant memory context.
    fn build_prompt(&self, obs: &Observation) -> String {
        let tools_desc = if self.tool_descriptions.is_empty() {
            "bash: Run shell commands\nweb_search: Search the web".to_string()
        } else {
            serde_json::to_string_pretty(&self.tool_descriptions).unwrap_or_else(|e| {
                tracing::error!(error = %e, "Failed to serialize tool descriptions");
                "Tools unavailable".into()
            })
        };

        let memory_hint = if obs.memory_ctx.is_empty() {
            String::new()
        } else {
            let memories: Vec<_> = obs
                .memory_ctx
                .iter()
                .map(|m| format!("- {}", m.content))
                .collect();
            format!("\nRelevant past experiences:\n{}\n", memories.join("\n"))
        };

        let mut prompt = String::new();
        prompt.push_str("You are a task planner. Decompose the user's goal into executable steps.\n\n");
        prompt.push_str("Available tools:\n");
        prompt.push_str(&tools_desc);
        prompt.push_str("\n\n");
        prompt.push_str("TOOL SELECTION GUIDE:\n");
        prompt.push_str("- reply: Use for conversation, questions, explanations, or any task that doesn't require external actions. This is the DEFAULT.\n");
        prompt.push_str("- bash: Use ONLY when shell commands are needed (installing packages, running scripts, system operations)\n");
        prompt.push_str("- read_file/write_file: Use ONLY when files must be read or written to disk\n");
        prompt.push_str("- web_search: Use ONLY when information not in your knowledge is needed\n");
        prompt.push_str("\nIMPORTANT: If the user just wants an answer, explanation, or conversation — use a single `reply` step.\n");
        prompt.push_str("Do NOT use file/shell tools to \"produce output\" — the reply tool IS the output.\n\n");
        prompt.push_str("Return ONLY a JSON array of steps. Each step must have:\n");
        prompt.push_str("- \"tool\": string — the tool name to use\n");
        prompt.push_str("- \"args\": object — parameters for the tool\n");
        prompt.push_str("- \"depends\": number[] — 0-based indices of steps this one depends on (empty array if none)\n");
        prompt.push_str("- \"candidates\": string[] — strategy names to consider for this step\n");
        prompt.push_str("\nData passing between steps:\n");
        prompt.push_str(
            "- Use {{step_N.output}} in args to reference the output of step N (0-based index)\n",
        );
        prompt.push_str("- Only reference steps listed in your \"depends\" array\n");
        prompt.push_str("- Example: if step 0 runs a command, step 1 can use {\"input\": \"{{step_0.output}}\"}\n");
        prompt.push_str("\nConstraints:\n");
        prompt.push_str("- Steps must be concrete and executable, not vague descriptions\n");
        prompt.push_str("- Dependencies must only reference earlier steps (lower indices)\n");
        prompt.push_str("- Prefer fewer, more powerful steps over many trivial ones\n");
        prompt.push('\n');
        prompt.push_str(&memory_hint);
        prompt.push_str("User goal: ");
        prompt.push_str(&obs.user_input);
        prompt.push_str("\n\nReturn ONLY the JSON array, no other text or explanation:");

        prompt
    }

    /// Stream LLM completion, printing tokens to stderr in real-time
    /// and collecting the full response.
    async fn complete_streaming(&self, prompt: String) -> Result<String> {
        let mut stream = self.llm.complete_stream(prompt).await?;
        let mut full = String::new();

        if let Some(ref tx) = self.event_tx {
            // TUI mode: send tokens through channel
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(token) => {
                        let _ = tx.send(AgentEvent::PlanStreamingToken {
                            token: token.clone(),
                        });
                        full.push_str(&token);
                    }
                    Err(e) => {
                        let _ = tx.send(AgentEvent::AgentError {
                            message: e.to_string(),
                        });
                        return Err(e);
                    }
                }
            }
        } else {
            // CLI mode: print to stderr
            eprint!("  \x1b[36m"); // cyan
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(token) => {
                        eprint!("{}", token);
                        full.push_str(&token);
                    }
                    Err(e) => {
                        eprintln!("\x1b[0m");
                        return Err(e);
                    }
                }
            }
            eprintln!("\x1b[0m"); // reset
        }
        Ok(full)
    }

    /// Extract JSON from LLM output that may be wrapped in markdown fences.
    fn extract_json(raw: &str) -> &str {
        let raw = raw.trim();
        // Strip ```json ... ``` fences
        if let Some(inner) = raw
            .strip_prefix("```json")
            .and_then(|s| s.strip_suffix("```"))
        {
            return inner.trim();
        }
        // Strip ``` ... ``` fences
        if let Some(inner) = raw.strip_prefix("```").and_then(|s| s.strip_suffix("```")) {
            return inner.trim();
        }
        raw
    }
}
