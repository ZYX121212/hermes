// crates/hermess-agent/src/mimo.rs
// Mixture-of-Agents (MiMo) — 多模型并行协作，在规划层运行 N 个模型，
// 由评判模型聚合最佳方案，提升复杂任务规划质量。
//
// 两种模式：
//   MiMoMode::PlanOnly   — 仅规划层并行，执行层用最佳方案单次执行（默认）
//   MiMoMode::FullAgent  — 完整 agent 循环并行，开销大但更彻底

use std::sync::Arc;
use std::time::Instant;

use agent_core::{Observation, Plan};
use llm::LlmAdapter;
use tokio::task::JoinSet;

/// MiMo execution mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiMoMode {
    /// Parallel planning only — N plans generated, best one executed.
    PlanOnly,
    /// Full parallel — N complete agent loops, results aggregated.
    FullAgent,
}

/// Aggregation strategy for combining multi-model outputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggregateStrategy {
    /// Judge model selects the single best plan.
    BestOfN,
    /// Judge model synthesizes a hybrid from all plans.
    Synthesize,
    /// Simple majority vote on tool choices (no judge call).
    MajorityVote,
}

/// A single model's output in the MiMo pipeline.
#[derive(Debug, Clone)]
pub struct MiMoCandidate {
    pub model_name: String,
    pub plan: Plan,
    pub latency_ms: u64,
}

/// Result of MiMo planning aggregation.
#[derive(Debug, Clone)]
pub struct MiMoResult {
    pub selected_plan: Plan,
    pub selected_model: String,
    pub candidates: Vec<MiMoCandidate>,
    pub reasoning: String,
    pub total_latency_ms: u64,
}

/// Multi-model orchestrator for Mixture-of-Agents planning.
///
/// Holds N worker models (the "mixture") and one judge model for aggregation.
/// All models share the same ToolRegistry for tool awareness.
pub struct MiMoRunner {
    /// Worker models that propose plans in parallel.
    pub workers: Vec<Arc<dyn LlmAdapter>>,
    /// Judge model that selects or synthesizes the best plan.
    pub judge: Arc<dyn LlmAdapter>,
    /// Aggregation strategy.
    pub strategy: AggregateStrategy,
    /// Minimum number of workers that must agree for majority vote.
    pub consensus_threshold: f64,
}

impl MiMoRunner {
    pub fn new(
        workers: Vec<Arc<dyn LlmAdapter>>,
        judge: Arc<dyn LlmAdapter>,
        strategy: AggregateStrategy,
    ) -> Self {
        Self {
            workers,
            judge,
            strategy,
            consensus_threshold: 0.5,
        }
    }

    /// Run MiMo planning: each worker proposes a plan in parallel,
    /// then the judge (or voting) selects the best one.
    pub async fn plan(
        &self,
        planner_factory: &Arc<dyn Fn(Arc<dyn LlmAdapter>) -> planner::Planner + Send + Sync>,
        obs: &Observation,
    ) -> anyhow::Result<MiMoResult> {
        let start = Instant::now();

        if self.workers.is_empty() {
            anyhow::bail!("MiMo requires at least one worker model");
        }

        // ── Phase 1: Parallel plan generation ──
        let mut js = JoinSet::new();
        for (i, worker) in self.workers.iter().enumerate() {
            let worker = Arc::clone(worker);
            let obs = obs.clone();
            let factory = Arc::clone(planner_factory);
            js.spawn(async move {
                let t0 = Instant::now();
                let planner = factory(Arc::clone(&worker) as Arc<dyn LlmAdapter>);
                let plan = planner.plan(obs).await;
                let latency_ms = t0.elapsed().as_millis() as u64;
                (i, worker, plan, latency_ms)
            });
        }

        let mut candidates: Vec<(usize, MiMoCandidate)> = Vec::new();
        while let Some(result) = js.join_next().await {
            match result {
                Ok((idx, worker, Ok(plan), latency_ms)) => {
                    let model_name = worker
                        .last_route_info()
                        .map(|ri| ri.routed_model)
                        .unwrap_or_else(|| format!("worker-{idx}"));
                    candidates.push((
                        idx,
                        MiMoCandidate {
                            model_name,
                            plan,
                            latency_ms,
                        },
                    ));
                }
                Ok((idx, _worker, Err(e), _)) => {
                    tracing::warn!(worker = idx, error = %e, "MiMo worker plan failed");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "MiMo worker task panicked");
                }
            }
        }

        if candidates.is_empty() {
            anyhow::bail!("All MiMo workers failed to produce a plan");
        }

        // Sort by original index for stable output
        candidates.sort_by_key(|(i, _)| *i);

        // ── Phase 2: Aggregation ──
        let (selected_idx, reasoning) = match self.strategy {
            AggregateStrategy::MajorityVote => {
                self.aggregate_majority(&candidates).await
            }
            AggregateStrategy::BestOfN => {
                self.aggregate_best_of_n(&candidates, obs).await
            }
            AggregateStrategy::Synthesize => {
                self.aggregate_synthesize(&candidates, obs).await
            }
        };

        let total_ms = start.elapsed().as_millis() as u64;
        let winner = &candidates[selected_idx];

        Ok(MiMoResult {
            selected_plan: winner.1.plan.clone(),
            selected_model: winner.1.model_name.clone(),
            candidates: candidates.into_iter().map(|(_, c)| c).collect(),
            reasoning,
            total_latency_ms: total_ms,
        })
    }

    /// Majority vote: count tool selections and pick the most common tool chain.
    async fn aggregate_majority(
        &self,
        candidates: &[(usize, MiMoCandidate)],
    ) -> (usize, String) {
        use std::collections::HashMap;

        // Build a fingerprint for each plan: sequence of tool names
        let fingerprints: Vec<Vec<String>> = candidates
            .iter()
            .map(|(_, c)| {
                c.plan
                    .steps
                    .iter()
                    .map(|s| s.tool.clone())
                    .collect::<Vec<_>>()
            })
            .collect();

        // Count occurrences of each fingerprint
        let mut counts: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
        for (i, fp) in fingerprints.iter().enumerate() {
            counts.entry(fp.clone()).or_default().push(i);
        }

        // Find the most common fingerprint
        let (best_fp, best_indices) = counts
            .into_iter()
            .max_by_key(|(_, indices)| indices.len())
            .unwrap_or_else(|| (fingerprints[0].clone(), vec![0]));

        let vote_count = best_indices.len();
        let total = candidates.len();
        let selected = best_indices[0];

        let reasoning = if vote_count as f64 / total as f64 >= self.consensus_threshold {
            format!(
                "多数投票: {}/{} 模型选择工具链 [{}]",
                vote_count,
                total,
                best_fp.join(" → ")
            )
        } else {
            format!(
                "多数投票 (未达共识阈值{}%): {}/{} 模型选择工具链 [{}], 回退到第一个候选",
                (self.consensus_threshold * 100.0) as u32,
                vote_count,
                total,
                best_fp.join(" → ")
            )
        };

        (selected, reasoning)
    }

    /// Best-of-N: judge model ranks candidates and picks the best one.
    async fn aggregate_best_of_n(
        &self,
        candidates: &[(usize, MiMoCandidate)],
        obs: &Observation,
    ) -> (usize, String) {
        if candidates.len() == 1 {
            return (
                0,
                "仅有一个候选方案，无需评判".to_string(),
            );
        }

        let prompt = build_judge_prompt(candidates, &obs.user_input, false);
        match self.judge.complete(prompt).await {
            Ok(response) => {
                let (idx, reason) = parse_judge_response(&response, candidates.len());
                (idx, reason)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Judge model call failed, using first candidate");
                (0, format!("评判模型调用失败: {e}, 回退到第一个候选"))
            }
        }
    }

    /// Synthesize: judge model combines the best elements from all plans.
    async fn aggregate_synthesize(
        &self,
        candidates: &[(usize, MiMoCandidate)],
        obs: &Observation,
    ) -> (usize, String) {
        if candidates.len() == 1 {
            return (
                0,
                "仅有一个候选方案，无需综合".to_string(),
            );
        }

        let prompt = build_judge_prompt(candidates, &obs.user_input, true);
        match self.judge.complete(prompt).await {
            Ok(response) => {
                let (idx, reason) = parse_judge_response(&response, candidates.len());
                (idx, reason)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Synthesize judge call failed, using first candidate");
                (0, format!("综合评判调用失败: {e}, 回退到第一个候选"))
            }
        }
    }
}

/// Build the judge prompt: present all candidate plans and ask the judge to
/// select (or synthesize) the best approach.
fn build_judge_prompt(
    candidates: &[(usize, MiMoCandidate)],
    user_input: &str,
    synthesize: bool,
) -> String {
    let mut descs = String::new();
    for (i, (_, c)) in candidates.iter().enumerate() {
        let steps_desc: Vec<String> = c
            .plan
            .steps
            .iter()
            .map(|s| {
                format!(
                    "  - {}: {} (依赖: {:?}, 候选: {:?})",
                    s.tool, s.args, s.depends, s.tool_candidates
                )
            })
            .collect();
        descs.push_str(&format!(
            "## 候选 {i} (模型: {})\n步骤数: {}\n{}\n\n",
            c.model_name,
            c.plan.steps.len(),
            steps_desc.join("\n")
        ));
    }

    if synthesize {
        format!(
            r#"你是多模型综合评判器。用户的问题是："{user_input}"。

以下是 {n} 个不同模型提出的方案。请综合各方案的最佳元素，选出最优方案并说明理由。

{descs}

请输出你的选择（按以下格式）：
SELECTED: <候选编号 0-{max_idx}>
REASON: <综合各方案后的评判理由>"#,
            n = candidates.len(),
            max_idx = candidates.len().saturating_sub(1),
        )
    } else {
        format!(
            r#"你是多模型评判器。用户的问题是："{user_input}"。

以下是 {n} 个不同模型提出的方案。请选出最优方案并说明理由。

{descs}

请输出你的选择（按以下格式）：
SELECTED: <候选编号 0-{max_idx}>
REASON: <评判理由>"#,
            n = candidates.len(),
            max_idx = candidates.len().saturating_sub(1),
        )
    }
}

/// Parse judge model response: extract SELECTED index and REASON text.
fn parse_judge_response(response: &str, num_candidates: usize) -> (usize, String) {
    let mut selected = 0usize;
    let mut reason = String::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed
            .strip_prefix("SELECTED:")
            .or_else(|| trimmed.strip_prefix("selected:"))
        {
            if let Ok(idx) = val.trim().parse::<usize>() {
                if idx < num_candidates {
                    selected = idx;
                }
            }
        }
        if let Some(val) = trimmed
            .strip_prefix("REASON:")
            .or_else(|| trimmed.strip_prefix("reason:"))
        {
            reason = val.trim().to_string();
        }
    }

    if reason.is_empty() {
        reason = format!(
            "评判模型未提供明确理由 (原始响应前 200 字符: {})",
            &response.chars().take(200).collect::<String>()
        );
    }

    (selected, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_judge_selects_valid_index() {
        let resp = "SELECTED: 2\nREASON: 候选2的工具链最简洁";
        let (idx, reason) = parse_judge_response(resp, 4);
        assert_eq!(idx, 2);
        assert!(reason.contains("最简洁"));
    }

    #[test]
    fn parse_judge_clamps_invalid_index() {
        let resp = "SELECTED: 99\nREASON: out of bounds";
        let (idx, _) = parse_judge_response(resp, 3);
        assert_eq!(idx, 0); // stays default, 99 is out of bounds
    }

    #[test]
    fn parse_judge_handles_lowercase() {
        let resp = "selected: 1\nreason: best approach";
        let (idx, reason) = parse_judge_response(resp, 3);
        assert_eq!(idx, 1);
        assert_eq!(reason, "best approach");
    }

    #[test]
    fn parse_judge_missing_fields() {
        let resp = "no structured output here";
        let (idx, reason) = parse_judge_response(resp, 3);
        assert_eq!(idx, 0);
        assert!(!reason.is_empty());
    }

    #[test]
    fn build_judge_prompt_includes_all_candidates() {
        use agent_core::{DependencyGraph, Step};
        let c1 = MiMoCandidate {
            model_name: "gpt-4".into(),
            plan: Plan {
                id: uuid::Uuid::new_v4(),
                steps: vec![Step {
                    id: uuid::Uuid::new_v4(),
                    tool: "bash".into(),
                    args: serde_json::json!({"cmd": "ls"}),
                    depends: vec![],
                    strategy: "parallel".into(),
                    tool_candidates: vec![],
                    delegable: false,
                }],
                dag: DependencyGraph::new(),
            },
            latency_ms: 100,
        };
        let c2 = MiMoCandidate {
            model_name: "claude".into(),
            plan: Plan {
                id: uuid::Uuid::new_v4(),
                steps: vec![Step {
                    id: uuid::Uuid::new_v4(),
                    tool: "read_file".into(),
                    args: serde_json::json!({"path": "src/main.rs"}),
                    depends: vec![],
                    strategy: "sequential".into(),
                    tool_candidates: vec![],
                    delegable: false,
                }],
                dag: DependencyGraph::new(),
            },
            latency_ms: 80,
        };
        let candidates = [(0, c1), (1, c2)];
        let prompt = build_judge_prompt(&candidates, "test task", false);
        assert!(prompt.contains("gpt-4"));
        assert!(prompt.contains("claude"));
        assert!(prompt.contains("候选 0"));
        assert!(prompt.contains("候选 1"));
        assert!(prompt.contains("test task"));
    }

    #[test]
    fn build_judge_synthesize_differs() {
        use agent_core::{DependencyGraph, Step};
        let c = MiMoCandidate {
            model_name: "m1".into(),
            plan: Plan {
                id: uuid::Uuid::new_v4(),
                steps: vec![Step {
                    id: uuid::Uuid::new_v4(),
                    tool: "bash".into(),
                    args: serde_json::json!({}),
                    depends: vec![],
                    strategy: "parallel".into(),
                    tool_candidates: vec![],
                    delegable: false,
                }],
                dag: DependencyGraph::new(),
            },
            latency_ms: 50,
        };
        let candidates = [(0, c)];
        let normal = build_judge_prompt(&candidates, "t", false);
        let synth = build_judge_prompt(&candidates, "t", true);
        assert!(synth.contains("综合"));
        assert!(!normal.contains("综合"));
    }
}
