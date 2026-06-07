use crate::feedback::FeedbackTracker;
use crate::models::{Classification, ModelEntry, RouteMode};

/// Three routing strategies, now aware of runtime model feedback.
pub struct RouteStrategy;

impl RouteStrategy {
    /// Pick a model name based on mode, classification, available models,
    /// and runtime feedback from prior LLM calls.
    pub fn decide(
        mode: &RouteMode,
        classification: &Classification,
        models: &[ModelEntry],
        feedback: &FeedbackTracker,
    ) -> Option<String> {
        if models.is_empty() {
            return None;
        }
        match mode {
            RouteMode::CostFirst => Self::cost_first(classification, models, feedback),
            RouteMode::QualityFirst => Self::quality_first(classification, models, feedback),
            RouteMode::LatencyFirst => Self::latency_first(classification, models, feedback),
        }
    }

    fn cost_first(
        classification: &Classification,
        models: &[ModelEntry],
        feedback: &FeedbackTracker,
    ) -> Option<String> {
        let min_cap = Self::min_capability_from_complexity(classification.complexity);
        models
            .iter()
            .filter(|m| feedback.adjusted_capability(&m.name, m.capability.reasoning) >= min_cap)
            .min_by(|a, b| {
                (a.cost_per_1m_input + a.cost_per_1m_output)
                    .partial_cmp(&(b.cost_per_1m_input + b.cost_per_1m_output))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|m| m.name.clone())
    }

    fn quality_first(
        _classification: &Classification,
        models: &[ModelEntry],
        feedback: &FeedbackTracker,
    ) -> Option<String> {
        models
            .iter()
            .max_by(|a, b| {
                let ca = feedback.adjusted_capability(&a.name, a.capability.reasoning);
                let cb = feedback.adjusted_capability(&b.name, b.capability.reasoning);
                ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|m| m.name.clone())
    }

    fn latency_first(
        classification: &Classification,
        models: &[ModelEntry],
        feedback: &FeedbackTracker,
    ) -> Option<String> {
        let min_cap = Self::min_capability_from_complexity(classification.complexity);
        models
            .iter()
            .filter(|m| feedback.adjusted_capability(&m.name, m.capability.reasoning) >= min_cap)
            .min_by_key(|m| feedback.adjusted_speed_ms(&m.name, m.capability.speed_ms))
            .map(|m| m.name.clone())
    }

    fn min_capability_from_complexity(complexity: f64) -> f64 {
        if complexity > 0.8 {
            0.7
        } else if complexity > 0.5 {
            0.4
        } else {
            0.1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelCapability;

    fn models() -> Vec<ModelEntry> {
        vec![
            ModelEntry {
                name: "cheap".into(),
                provider: "openai".into(),
                base_url: String::new(),
                api_key: "k".into(),
                cost_per_1m_input: 0.3,
                cost_per_1m_output: 0.6,
                capability: ModelCapability {
                    reasoning: 0.4,
                    coding: 0.5,
                    creative: 0.3,
                    knowledge: 0.5,
                    speed_ms: 50,
                },
                tags: vec!["fast".into()],
            },
            ModelEntry {
                name: "mid".into(),
                provider: "openai".into(),
                base_url: String::new(),
                api_key: "k".into(),
                cost_per_1m_input: 1.0,
                cost_per_1m_output: 4.0,
                capability: ModelCapability {
                    reasoning: 0.7,
                    coding: 0.8,
                    creative: 0.6,
                    knowledge: 0.7,
                    speed_ms: 200,
                },
                tags: vec!["general".into()],
            },
            ModelEntry {
                name: "smart".into(),
                provider: "anthropic".into(),
                base_url: String::new(),
                api_key: "k".into(),
                cost_per_1m_input: 15.0,
                cost_per_1m_output: 75.0,
                capability: ModelCapability {
                    reasoning: 0.95,
                    coding: 0.9,
                    creative: 0.8,
                    knowledge: 0.9,
                    speed_ms: 2000,
                },
                tags: vec!["reasoning".into()],
            },
        ]
    }

    fn empty_feedback() -> FeedbackTracker {
        FeedbackTracker::new()
    }

    #[test]
    fn cost_first_low_complexity_picks_cheapest() {
        let cls = Classification {
            complexity: 0.2,
            is_short_hard: false,
            suggested_tags: vec![],
        };
        let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models(), &empty_feedback());
        assert_eq!(pick, Some("cheap".into()));
    }

    #[test]
    fn cost_first_high_complexity_picks_balanced() {
        let cls = Classification {
            complexity: 0.9,
            is_short_hard: false,
            suggested_tags: vec![],
        };
        let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models(), &empty_feedback());
        assert_eq!(pick, Some("mid".into()));
    }

    #[test]
    fn quality_first_always_picks_smartest() {
        let cls = Classification {
            complexity: 0.1,
            is_short_hard: false,
            suggested_tags: vec![],
        };
        let pick =
            RouteStrategy::decide(&RouteMode::QualityFirst, &cls, &models(), &empty_feedback());
        assert_eq!(pick, Some("smart".into()));
    }

    #[test]
    fn latency_first_low_complexity_picks_fastest() {
        let cls = Classification {
            complexity: 0.2,
            is_short_hard: false,
            suggested_tags: vec![],
        };
        let pick =
            RouteStrategy::decide(&RouteMode::LatencyFirst, &cls, &models(), &empty_feedback());
        assert_eq!(pick, Some("cheap".into()));
    }

    #[test]
    fn latency_first_high_complexity_excludes_slow_but_cheap() {
        let cls = Classification {
            complexity: 0.9,
            is_short_hard: false,
            suggested_tags: vec![],
        };
        let pick =
            RouteStrategy::decide(&RouteMode::LatencyFirst, &cls, &models(), &empty_feedback());
        assert_eq!(pick, Some("mid".into()));
    }

    #[test]
    fn empty_models_returns_none() {
        let cls = Classification::default();
        assert_eq!(
            RouteStrategy::decide(&RouteMode::CostFirst, &cls, &[], &empty_feedback()),
            None
        );
    }

    #[test]
    fn feedback_penalizes_unreliable_model() {
        let fb = FeedbackTracker::new();
        // Make "smart" look unreliable
        fb.record_failure("smart", 5000);
        fb.record_failure("smart", 5000);

        let cls = Classification {
            complexity: 0.1,
            is_short_hard: false,
            suggested_tags: vec![],
        };
        // quality-first should now prefer "mid" over the degraded "smart"
        let pick = RouteStrategy::decide(&RouteMode::QualityFirst, &cls, &models(), &fb);
        assert_eq!(pick, Some("mid".into()));
    }
}
