use std::sync::Arc;

use crate::classifier::ComplexityClassifier;
use crate::config::OptimizerConfig;
use crate::feedback::FeedbackTracker;
use crate::models::{Classification, RouteMode, RouteTarget, RoutingDecision};
use crate::registry::ModelRegistry;
use crate::shg::ShgDetector;
use crate::strategy::RouteStrategy;

/// Ties SHG → Classifier → Strategy → Decision into a single pipeline.
pub struct DecisionEngine {
    shg: ShgDetector,
    classifier: ComplexityClassifier,
    optimizer_config: OptimizerConfig,
    registry: Arc<ModelRegistry>,
    feedback: Arc<FeedbackTracker>,
}

impl DecisionEngine {
    pub fn new(
        shg: ShgDetector,
        classifier: ComplexityClassifier,
        optimizer_config: OptimizerConfig,
        registry: Arc<ModelRegistry>,
        feedback: Arc<FeedbackTracker>,
    ) -> Self {
        Self {
            shg,
            classifier,
            optimizer_config,
            registry,
            feedback,
        }
    }

    /// Full routing pipeline: SHG → classify → strategy → decision.
    pub async fn decide(&self, prompt: &str, mode: &RouteMode) -> RoutingDecision {
        // 1. SHG check (<1ms)
        if let Some(force_model) = self.shg.check(prompt) {
            return RoutingDecision {
                target: RouteTarget::Single(force_model),
                reasoning: "SHG: short-hard request, bypassing classifier".into(),
            };
        }

        // 2. Classify (<50ms, with timeout fallback)
        let classification = self.classifier.classify(prompt).await;

        // 3. Strategy decision (now feedback-aware)
        let models = self.registry.all().to_vec();
        let model = RouteStrategy::decide(mode, &classification, &models, &self.feedback)
            .unwrap_or_else(|| {
                tracing::warn!("No model matched strategy, using first available");
                models.first().map(|m| m.name.clone()).unwrap_or_default()
            });

        // 4. If optimizer is enabled and complexity is mid-range, decompose
        if self.optimizer_config.decompose_enabled
            && classification.complexity > 0.3
            && classification.complexity < 0.9
        {
            let small = RouteStrategy::decide(
                &RouteMode::CostFirst,
                &Classification {
                    complexity: 0.2,
                    ..Classification::default()
                },
                &models,
                &self.feedback,
            );
            if let Some(small_model) = small {
                if small_model != model {
                    return RoutingDecision {
                        target: RouteTarget::Decomposed {
                            critical: model,
                            regular: small_model,
                        },
                        reasoning: format!(
                            "Optimizer: complexity={:.2}, decomposed critical→large, regular→small",
                            classification.complexity
                        ),
                    };
                }
            }
        }

        RoutingDecision {
            target: RouteTarget::Single(model),
            reasoning: format!("complexity={:.2}, mode={mode}", classification.complexity),
        }
    }
}
