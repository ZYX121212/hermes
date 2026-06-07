// crates/planner/src/planner.rs
// LLM-driven task planner: decomposes a user goal into executable steps
// arranged in a DAG, with optimal strategies selected from the evolution engine.
use std::path::{Path, PathBuf};
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
    /// Project skill context discovered from `.hermess/skills`.
    skill_context: ProjectSkillContext,
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
            skill_context: ProjectSkillContext::discover(),
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

    /// Override project skill context, mainly for tests and embedding hosts.
    pub fn set_skill_context(&mut self, skill_context: String) {
        self.skill_context = ProjectSkillContext::from_text(skill_context);
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
            .map(|(i, mut s)| {
                Self::enrich_tool_args(&s.tool, &mut s.args, &obs.user_input);
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
    pub async fn replan(
        &self,
        obs: &Observation,
        failed: &agent_core::ExecutionResult,
    ) -> Result<Plan> {
        self.emit(AgentEvent::PlanPhaseStarted);

        let failures: Vec<String> = failed
            .outputs
            .iter()
            .filter(|o| !o.success)
            .map(|o| format!("- step {} (tool={}): {}", o.step_id, o.tool, o.content))
            .collect();

        // ── Conversation history for context in retry ──
        let history_text = if obs.conversation_history.is_empty() {
            String::new()
        } else {
            let entries: Vec<String> = obs
                .conversation_history
                .iter()
                .map(|(q, a)| format!("用户: {}\n助手: {}", q, a))
                .collect();
            format!("\n## 对话历史\n{}\n", entries.join("\n\n"))
        };

        // ── Recent evolution insights ──
        let insights_text = if obs.recent_insights.is_empty() {
            String::new()
        } else {
            let items: Vec<String> = obs
                .recent_insights
                .iter()
                .map(|i| format!("- {}", i))
                .collect();
            format!("\n## 最近学习到的经验\n{}\n", items.join("\n"))
        };

        let skill_text = self.skill_context.render();
        let prompt = format!(
            "你之前的执行计划失败了，需要换个思路。\n\n\
             ## 失败的步骤\n{}\n\
             ## 原始任务\n{}\n\
             {}\
             {}\
             {}\
             ## 可用工具\n{}\n\n\
             提出一个替代方案，使用不同的工具或方法，避免使用已经失败的工具。\
             只返回 JSON 数组，不要其他文字:\n",
            failures.join("\n"),
            obs.user_input,
            history_text,
            insights_text,
            skill_text,
            serde_json::to_string_pretty(&self.tool_descriptions).unwrap_or_default(),
        );

        let raw = if self.streaming {
            self.complete_streaming(prompt).await?
        } else {
            self.llm.complete(prompt).await?
        };
        let raw = Self::extract_json(&raw);
        let specs: Vec<StepSpec> =
            serde_json::from_str(raw).map_err(|e| anyhow::anyhow!("重规划解析失败: {e}"))?;
        if specs.is_empty() {
            return Err(anyhow::anyhow!("重规划未能生成有效步骤"));
        }

        let step_ids: Vec<Uuid> = specs.iter().map(|_| Uuid::new_v4()).collect();
        let dag = build_dag(&specs, &step_ids)?;

        let steps: Vec<Step> = specs
            .into_iter()
            .enumerate()
            .map(|(i, mut s)| {
                Self::enrich_tool_args(&s.tool, &mut s.args, &obs.user_input);
                let candidates: Vec<&str> = s.candidates.iter().map(|c| c.as_str()).collect();
                let strategy = self
                    .evolution
                    .best_strategy(&candidates)
                    .unwrap_or("default".into());
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

        Ok(Plan {
            id: Uuid::new_v4(),
            steps,
            dag,
        })
    }

    /// Send an event if a sender is configured.
    fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            if tx.send(event).is_err() {
                tracing::warn!("Planner event channel closed");
            }
        }
    }

    /// Build a planning prompt that includes system persona, conversation history,
    /// long-term memory, evolution insights, and tool descriptions.
    fn build_prompt(&self, obs: &Observation) -> String {
        let tools_desc = if self.tool_descriptions.is_empty() {
            "bash: Run shell commands\nweb_search: Search the web".to_string()
        } else {
            serde_json::to_string_pretty(&self.tool_descriptions).unwrap_or_else(|e| {
                tracing::error!(error = %e, "Failed to serialize tool descriptions");
                "Tools unavailable".into()
            })
        };

        // ── Conversation history as structured transcript ──
        let history_text = if obs.conversation_history.is_empty() {
            String::new()
        } else {
            let entries: Vec<String> = obs
                .conversation_history
                .iter()
                .map(|(q, a)| format!("用户: {}\n助手: {}", q, a))
                .collect();
            format!("\n## 对话历史\n{}\n", entries.join("\n\n"))
        };

        // ── Relevant long-term memories ──
        let memory_text = if obs.memory_ctx.is_empty() {
            String::new()
        } else {
            let memories: Vec<_> = obs
                .memory_ctx
                .iter()
                .map(|m| format!("- {}", m.content))
                .collect();
            format!("\n## 相关历史经验\n{}\n", memories.join("\n"))
        };

        // ── Recent evolution insights ──
        let insights_text = if obs.recent_insights.is_empty() {
            String::new()
        } else {
            let items: Vec<String> = obs
                .recent_insights
                .iter()
                .map(|i| format!("- {}", i))
                .collect();
            format!("\n## 最近学习到的经验\n{}\n", items.join("\n"))
        };

        let mut prompt = String::new();

        // ── System prompt ──
        prompt.push_str("你是一个智能任务助手 Hermess。你正处于一段持续的多轮对话中，有能力记住并引用之前的对话内容。\n\n");
        prompt.push_str("## 能力\n");
        prompt.push_str("- 你可以将任务分解为可执行的步骤，也可以直接回复对话\n");
        prompt.push_str("- 当用户只是提问、闲聊或寻求建议时，使用单个 reply 步骤直接回复\n");
        prompt.push_str("- 只有在需要外部操作（搜索、文件、命令）时才使用工具\n\n");

        // ── Context sections ──
        prompt.push_str(&history_text);
        prompt.push_str(&memory_text);
        prompt.push_str(&insights_text);
        prompt.push_str(&self.skill_context.render());

        // ── Tools ──
        prompt.push_str("## 可用工具\n");
        prompt.push_str(&tools_desc);
        prompt.push_str("\n\n");

        // ── Tool selection guide ──
        prompt.push_str("## 工具选择指南\n");
        prompt.push_str("- reply: 对话、回答问题、解释说明——直接回复用户，这是默认选择\n");
        prompt.push_str("- bash: 仅当 Shell 命令必须时使用（安装软件、运行脚本、系统操作）\n");
        prompt.push_str("- read_file/write_file: 仅当需要读写磁盘文件时使用\n");
        prompt.push_str("- web_search: 仅当需要获取训练知识之外的信息时使用\n");
        prompt
            .push_str("- financial_query: 查询股票行情、指数列表、基金净值、宏观指标等金融数据\n");
        prompt.push_str("- ftshare_market_data: 查询 FTShare/market.ft.tech 的完整金融数据。用户用自然语言查询指数/股票 K线、日期、行情时，必须在 args.query 中原样放入当前任务；例如 {\"subskill\":\"index-ohlcs\",\"query\":\"请获取上证指数的k线数据 20260603的\",\"args\":{}}。上证指数/沪指是指数，应使用 index-ohlcs，不要误用 stock-ohlcs；未指定周期时默认日K DAY1。\n");
        prompt.push_str("\n**重要**: 如果用户只是要回答或解释，使用单个 reply 步骤。不要为了'输出'而使用文件/shell 工具。\n\n");

        // ── JSON format ──
        prompt.push_str("## 输出格式\n");
        prompt.push_str("返回一个 JSON 数组，每个步骤对象包含:\n");
        prompt.push_str("- \"tool\": string — 工具名称\n");
        prompt.push_str("- \"args\": object — 工具参数\n");
        prompt.push_str("- \"depends\": number[] — 依赖的步骤索引（0-based），无依赖则为空数组\n");
        prompt.push_str("- \"candidates\": string[] — 备选策略列表\n");
        prompt.push_str("- \"delegable\": bool — 是否启动子Agent自主推理（默认false）\n\n");
        prompt.push_str("delegable 使用规则:\n");
        prompt.push_str("- true: 任务复杂，需要子Agent独立规划多步执行（如\"研究并撰写报告\"、\"对比分析数据\"）\n");
        prompt.push_str(
            "- false: 简单工具调用，直接执行即可（如\"读文件\"、\"搜索网页\"、\"查询金融数据\"）\n",
        );
        prompt.push_str("- 约束: delegable 步骤绝不能依赖其他步骤的输出（depends必须为空）\n");
        prompt.push_str("- 约束: delegable 步骤只使用描述性工具名（如\"research\"、\"analyze\"），不指定具体工具\n\n");
        prompt.push_str("步骤间数据传递:\n");
        prompt.push_str("- 使用 {{step_N.output}} 引用第 N 步的输出\n");
        prompt.push_str("- 只能引用 depends 中声明的步骤\n\n");
        prompt.push_str("约束:\n");
        prompt.push_str("- 步骤必须具体可执行\n");
        prompt.push_str("- 依赖只能引用前面（索引更小）的步骤\n");
        prompt.push_str("- 优先使用更少但更强大的步骤\n\n");

        // ── Current task ──
        prompt.push_str("## 当前任务\n");
        prompt.push_str(&obs.user_input);
        prompt.push_str("\n\n只返回 JSON 数组，不要其他文字:");

        prompt
    }

    fn enrich_tool_args(tool: &str, args: &mut serde_json::Value, user_input: &str) {
        if tool != "ftshare_market_data" {
            return;
        }
        if !args.is_object() {
            *args = serde_json::json!({});
        }
        if let Some(obj) = args.as_object_mut() {
            obj.entry("query".to_string())
                .or_insert_with(|| serde_json::Value::String(user_input.to_string()));
        }
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

#[derive(Debug, Clone, Default)]
struct ProjectSkillContext {
    text: String,
}

impl ProjectSkillContext {
    const MAX_BODY_CHARS: usize = 1800;
    const MAX_TOTAL_CHARS: usize = 8000;

    fn discover() -> Self {
        let cwd = match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(e) => {
                tracing::debug!(error = %e, "Cannot discover project skills without current dir");
                return Self::default();
            }
        };
        Self::discover_from(cwd)
    }

    fn discover_from(start_dir: PathBuf) -> Self {
        let mut current = start_dir.as_path();
        loop {
            let candidate = current.join(".hermess").join("skills");
            if candidate.is_dir() {
                return Self::from_dir(&candidate);
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => return Self::default(),
            }
        }
    }

    fn from_dir(dir: &Path) -> Self {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "Cannot read project skills");
                return Self::default();
            }
        };

        let mut skill_docs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let skill_md = if path.is_dir() {
                path.join("SKILL.md")
            } else if path.is_file()
                && path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md")
            {
                path
            } else {
                continue;
            };

            match Self::read_skill_md(&skill_md) {
                Some(doc) => skill_docs.push(doc),
                None => {
                    tracing::warn!(path = %skill_md.display(), "Skipping unreadable project skill")
                }
            }
        }

        skill_docs.sort_by(|a, b| a.name.cmp(&b.name));
        Self::from_text(Self::render_docs(&skill_docs))
    }

    fn from_text(text: String) -> Self {
        Self {
            text: Self::truncate_owned(text, Self::MAX_TOTAL_CHARS),
        }
    }

    fn read_skill_md(path: &Path) -> Option<SkillDoc> {
        let content = std::fs::read_to_string(path).ok()?;
        let (frontmatter, body) =
            Self::split_frontmatter(&content).unwrap_or(("", content.as_str()));
        let name = Self::frontmatter_value(frontmatter, "name")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(ToOwned::to_owned)
            })?;
        let description = Self::frontmatter_value(frontmatter, "description").unwrap_or_default();
        Some(SkillDoc {
            name,
            description,
            body: Self::select_body_excerpt(&content, body),
        })
    }

    fn render_docs(docs: &[SkillDoc]) -> String {
        if docs.is_empty() {
            return String::new();
        }

        let mut out = String::from(
            "## 项目 Skills\n\
             下面是当前项目 `.hermess/skills` 中自动读取的技能说明。遇到相关任务时必须按这些规则选择工具和参数；不要凭空编造未定义参数。\n\n",
        );
        for doc in docs {
            out.push_str(&format!("### {}\n", doc.name));
            if !doc.description.is_empty() {
                out.push_str(&doc.description);
                out.push('\n');
            }
            if !doc.body.is_empty() {
                out.push_str(&doc.body);
                out.push('\n');
            }
            if doc.name.to_ascii_lowercase().contains("ftshare") {
                out.push_str(
                    "- Hermess 工具调用规则：金融行情、A股/港股/指数/基金/K线/宏观数据优先使用 `ftshare_market_data`。\n\
                     - 自然语言请求必须把用户原文放入 `args.query`，让 wrapper 自动解析代码、日期、span 和参数修复。\n\
                     - 用户问上证指数/沪指/上证综指的 K 线时，使用 `subskill: index-ohlcs`；不要用 `stock-ohlcs`。\n\
                     - 上证指数默认代码是 `000001.XSHG`；K 线未指定周期时默认 `DAY1`；YYYYMMDD 日期应交给 wrapper 解析为 `until_ts_ms`，不要传官方脚本不支持的 `date` 参数。\n",
                );
            }
            out.push('\n');
        }
        out
    }

    fn render(&self) -> String {
        if self.text.trim().is_empty() {
            String::new()
        } else {
            format!("{}\n", self.text)
        }
    }

    fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
        let trimmed = content.trim_start();
        let after_first = trimmed
            .strip_prefix("---\n")
            .or_else(|| trimmed.strip_prefix("---\r\n"))?;
        let end = after_first
            .find("\n---")
            .or_else(|| after_first.find("\r\n---"))?;
        let frontmatter = &after_first[..end];
        let delim_len = if after_first[end..].starts_with("\n---\r\n") {
            6
        } else {
            5
        };
        let body = after_first.get(end + delim_len..).unwrap_or_default();
        Some((frontmatter, body))
    }

    fn frontmatter_value(frontmatter: &str, key: &str) -> Option<String> {
        let prefix = format!("{key}:");
        let line = frontmatter
            .lines()
            .find(|line| line.trim_start().starts_with(&prefix))?;
        let value = line.trim_start()[prefix.len()..].trim();
        Some(value.trim_matches('"').trim_matches('\'').to_string())
    }

    fn select_body_excerpt(full_content: &str, body: &str) -> String {
        let mut excerpts = Vec::new();
        let lower_full = full_content.to_ascii_lowercase();
        if lower_full.contains("ftshare") || body.contains("index-ohlcs") {
            let needles = [
                "## 指数",
                "index-ohlcs",
                "名称→代码映射",
                "stock-ohlcs",
                "until_ts_ms",
                "用户经常给出中文名称",
            ];
            for needle in needles {
                if let Some(snippet) = Self::snippet_around(body, needle, 900) {
                    excerpts.push(snippet);
                }
            }
        }

        if excerpts.is_empty() {
            excerpts.push(Self::truncate_owned(
                body.trim().to_string(),
                Self::MAX_BODY_CHARS,
            ));
        }

        let mut joined = excerpts.join("\n...\n");
        joined = Self::truncate_owned(joined, Self::MAX_BODY_CHARS);
        joined
    }

    fn snippet_around(haystack: &str, needle: &str, max_chars: usize) -> Option<String> {
        let pos = haystack.find(needle)?;
        let half = max_chars / 2;
        let start = Self::floor_char_boundary(haystack, pos.saturating_sub(half));
        let end =
            Self::ceil_char_boundary(haystack, (pos + needle.len() + half).min(haystack.len()));
        Some(haystack[start..end].trim().to_string())
    }

    fn truncate_owned(s: String, max_chars: usize) -> String {
        if s.chars().count() <= max_chars {
            return s;
        }
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("\n...(已截断)");
        out
    }

    fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }

    fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
        while idx < s.len() && !s.is_char_boundary(idx) {
            idx += 1;
        }
        idx
    }
}

#[derive(Debug, Clone)]
struct SkillDoc {
    name: String,
    description: String,
    body: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::Observation;
    use async_trait::async_trait;
    use futures::stream;
    use llm::LlmAdapter;
    use memory::MockMemoryStore;

    struct MockLlm;

    #[async_trait]
    impl LlmAdapter for MockLlm {
        async fn complete(&self, _prompt: String) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        async fn complete_stream(
            &self,
            _prompt: String,
        ) -> anyhow::Result<Box<dyn futures::Stream<Item = anyhow::Result<String>> + Unpin + Send>>
        {
            Ok(Box::new(stream::iter(Vec::<anyhow::Result<String>>::new())))
        }

        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0; 4])
        }
    }

    fn test_planner() -> Planner {
        let memory = Arc::new(MockMemoryStore::new());
        let evolution = Arc::new(EvolutionEngine::new(0.1, memory));
        Planner::new(Arc::new(MockLlm), evolution)
    }

    #[test]
    fn project_skill_context_loads_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".hermess").join("skills");
        let skill_dir = skills_dir.join("ftshare-market-data");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: FTShare-market-data\ndescription: 金融数据技能\n---\n\
             ## 指数\n\
             index-ohlcs 查询指数 K 线。\n\
             用户经常给出中文名称，如上证指数。\n\
             stock-ohlcs 是股票 K 线。\n\
             until_ts_ms 是截止时间戳。\n",
        )
        .unwrap();

        let ctx = ProjectSkillContext::discover_from(tmp.path().to_path_buf()).render();
        assert!(ctx.contains("## 项目 Skills"));
        assert!(ctx.contains("FTShare-market-data"));
        assert!(ctx.contains("金融数据技能"));
        assert!(ctx.contains("index-ohlcs"));
        assert!(ctx.contains("ftshare_market_data"));
        assert!(ctx.contains("000001.XSHG"));
    }

    #[test]
    fn planner_prompt_includes_skill_context() {
        let mut planner = test_planner();
        planner.set_skill_context(
            "## 项目 Skills\n### FTShare-market-data\nindex-ohlcs uses `ftshare_market_data`."
                .to_string(),
        );
        let obs = Observation {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            user_input: "请获取上证指数的k线数据 20260603的".to_string(),
            env_state: serde_json::json!({}),
            memory_ctx: vec![],
            conversation_history: vec![],
            recent_insights: vec![],
        };

        let prompt = planner.build_prompt(&obs);
        assert!(prompt.contains("## 项目 Skills"));
        assert!(prompt.contains("FTShare-market-data"));
        assert!(prompt.contains("index-ohlcs uses `ftshare_market_data`"));
        assert!(prompt.contains("请获取上证指数的k线数据 20260603的"));
    }
}
