// crates/reflector/src/reflector.rs
// Reflector: evaluates execution results and generates Insights
// that feed into the evolution engine.
use std::sync::Arc;

use agent_core::{ExecutionResult, Insight};
use anyhow::Result;
use evolution::Scorer;
use llm::LlmAdapter;

use crate::attribution::attribute_errors;

/// Reflector: evaluates execution results and generates Insights
/// that feed into the evolution engine.
///
/// Uses a Scorer for structured evaluation and optionally calls
/// the LLM for semantic error attribution on failures.
pub struct Reflector {
    llm: Arc<dyn LlmAdapter>,
    scorer: Scorer,
}

impl Reflector {
    /// Create a new Reflector with a default Scorer configuration.
    pub fn new(llm: Arc<dyn LlmAdapter>) -> Self {
        Self {
            llm,
            scorer: Scorer::default(),
        }
    }

    /// Create a new Reflector with a custom Scorer.
    pub fn with_scorer(llm: Arc<dyn LlmAdapter>, scorer: Scorer) -> Self {
        Self { llm, scorer }
    }

    /// Reflect on an execution result to produce an Insight.
    /// Scores the result, then uses LLM attribution for failures.
    pub async fn reflect(&self, result: &ExecutionResult) -> Result<Insight> {
        // 1. Structured scoring
        let score = self.scorer.score(result);

        // 2. Use LLM for semantic attribution on failure (save tokens on success)
        let lesson = if score < 0.0 {
            let prompt = format!(
                "Analyze this failed execution and write a one-sentence lesson.\n\
                 Execution: {}\n\
                 Duration: {}ms\n\
                 Step outputs:\n{}\n\n\
                 What went wrong and how should we adjust the strategy?",
                if result.success { "SUCCESS" } else { "FAILURE" },
                result.duration_ms,
                result
                    .outputs
                    .iter()
                    .map(|o| format!(
                        "  [{}] {}: {}",
                        if o.success { "OK" } else { "FAIL" },
                        o.step_id,
                        o.content
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            match self.llm.complete(prompt).await {
                Ok(lesson_text) => lesson_text,
                Err(e) => {
                    tracing::warn!("LLM attribution failed: {e}, using heuristic");
                    attribute_errors(result)
                }
            }
        } else {
            format!(
                "Strategy succeeded with score {:.2}. {}steps, {}ms.",
                score,
                if result.success { "" } else { "Partial " },
                result.duration_ms,
            )
        };

        // 3. Vectorize the lesson for long-term memory
        let embedding = match self.llm.embed(&lesson).await {
            Ok(emb) => emb,
            Err(e) => {
                tracing::warn!("Embedding failed: {e}, using zero vector");
                vec![0.0_f32; 1024]
            }
        };

        Ok(Insight {
            strategy_id: result.strategy_id(),
            score,
            embedding,
            lesson,
        })
    }
}
