// crates/evolution/src/scorer.rs
use agent_core::ExecutionResult;

/// Pure-function scorer: evaluates execution results without side effects.
pub struct Scorer {
    pub success_weight: f64,
    pub latency_weight: f64,
    pub quality_weight: f64,
    pub latency_target_ms: u64,
}

impl Default for Scorer {
    fn default() -> Self {
        Self {
            success_weight: 0.6,
            latency_weight: 0.2,
            quality_weight: 0.2,
            latency_target_ms: 2000,
        }
    }
}

impl Scorer {
    /// Compute a score in [-1.0, 1.0] for an execution result.
    pub fn score(&self, result: &ExecutionResult) -> f64 {
        let success = if result.success { 1.0 } else { -1.0 };
        let latency = 1.0
            - (result.duration_ms as f64 / self.latency_target_ms as f64)
                .min(1.0);
        let quality = self.measure_quality(result);

        self.success_weight * success
            + self.latency_weight * latency
            + self.quality_weight * quality
    }

    fn measure_quality(&self, result: &ExecutionResult) -> f64 {
        let total = result.outputs.len();
        if total == 0 {
            return 0.0;
        }
        let successes = result.outputs.iter().filter(|o| o.success).count();
        successes as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::StepOutput;
    use uuid::Uuid;

    #[test]
    fn test_score_success() {
        let scorer = Scorer::default();
        let result = ExecutionResult {
            plan_id: Uuid::new_v4(),
            outputs: vec![StepOutput {
                step_id: Uuid::new_v4(),
                success: true,
                content: "ok".into(),
                duration_ms: 1000,
            }],
            success: true,
            duration_ms: 1000,
        };
        let s = scorer.score(&result);
        assert!(s > 0.0, "successful result should score positive, got {s}");
    }

    #[test]
    fn test_score_failure() {
        let scorer = Scorer::default();
        let result = ExecutionResult {
            plan_id: Uuid::new_v4(),
            outputs: vec![],
            success: false,
            duration_ms: 5000,
        };
        let s = scorer.score(&result);
        assert!(s < 0.0, "failed result should score negative, got {s}");
    }

    #[test]
    fn test_score_timeout() {
        let scorer = Scorer::default();
        let result = ExecutionResult {
            plan_id: Uuid::new_v4(),
            outputs: vec![StepOutput {
                step_id: Uuid::new_v4(),
                success: true,
                content: "slow".into(),
                duration_ms: 5000,
            }],
            success: true,
            duration_ms: 5000,
        };
        let s = scorer.score(&result);
        // Should be lower than a fast success (which would score 1.0)
        assert!(s < 1.0, "slow result should score lower than perfect, got {s}");
    }
}
