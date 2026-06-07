use std::sync::Arc;
use tokio::time::{timeout, Duration};

use crate::config::ClassifierConfig;
use crate::metrics::RouteMetrics;
use crate::models::Classification;
use crate::registry::ModelRegistry;

/// Uses a lightweight LLM to classify request complexity.
#[derive(Clone)]
pub struct ComplexityClassifier {
    config: ClassifierConfig,
    registry: Arc<ModelRegistry>,
    metrics: Option<Arc<RouteMetrics>>,
    domain_context: Option<String>,
}

impl ComplexityClassifier {
    pub fn new(config: ClassifierConfig, registry: Arc<ModelRegistry>) -> Self {
        Self {
            config,
            registry,
            metrics: None,
            domain_context: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<RouteMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn with_domain_context(mut self, ctx: String) -> Self {
        if !ctx.is_empty() {
            self.domain_context = Some(ctx);
        }
        self
    }

    /// Classify a prompt. Returns default classification on timeout or error.
    pub async fn classify(&self, prompt: &str) -> Classification {
        let classifier_model = match self.registry.get(&self.config.model) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    model = %self.config.model,
                    "Classifier model not found in registry, using default classification"
                );
                return Classification::default();
            }
        };

        let mut classification_prompt = String::new();

        // Prepend project domain context from skills if available
        if let Some(ref ctx) = self.domain_context {
            classification_prompt.push_str("Project domain context:\n");
            classification_prompt.push_str(ctx);
            classification_prompt.push_str("\n\n");
        }

        classification_prompt.push_str(&format!(
            "Classify this request. Respond with ONLY valid JSON, no other text.\n\
             {{\"complexity\": <0.0-1.0>, \"tags\": [\"<tag1>\", \"<tag2>\"]}}\n\
             complexity: 0.0=trivial, 1.0=extremely complex reasoning needed.\n\
             tags from: reasoning, coding, creative, knowledge, general.\n\n\
             Request: {prompt}"
        ));

        let adapter_result = self.build_adapter(classifier_model);
        let adapter = match adapter_result {
            Some(a) => a,
            None => return Classification::default(),
        };

        let future = adapter.complete(classification_prompt);
        match timeout(Duration::from_millis(self.config.timeout_ms), future).await {
            Ok(Ok(response)) => {
                if let Some(ref m) = self.metrics {
                    m.inc_classifier_ok();
                }
                self.parse_classification(&response)
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Classifier LLM call failed, using default");
                if let Some(ref m) = self.metrics {
                    m.inc_classifier_fallback();
                }
                Classification::default()
            }
            Err(_elapsed) => {
                tracing::debug!("Classifier timed out after {}ms", self.config.timeout_ms);
                if let Some(ref m) = self.metrics {
                    m.inc_classifier_timeout();
                }
                Classification::default()
            }
        }
    }

    fn build_adapter(&self, model: &crate::models::ModelEntry) -> Option<Box<dyn llm::LlmAdapter>> {
        match model.provider.as_str() {
            "openai" | "deepseek" => Some(Box::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: model.api_key.clone(),
                model: model.name.clone(),
                max_tokens: 128,
                base_url: if model.base_url.is_empty() {
                    "https://api.openai.com/v1".into()
                } else {
                    model.base_url.clone()
                },
            }))),
            "anthropic" => Some(Box::new(llm::AnthropicAdapter::new(
                &llm::AnthropicConfig {
                    api_key: model.api_key.clone(),
                    model: model.name.clone(),
                    max_tokens: 128,
                },
            ))),
            other => {
                tracing::warn!(provider = %other, "Unknown classifier provider");
                None
            }
        }
    }

    fn parse_classification(&self, raw: &str) -> Classification {
        let trimmed = raw.trim();
        let json_str = trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(val) => {
                let complexity = val["complexity"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
                let suggested_tags: Vec<String> = val["tags"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Classification {
                    complexity,
                    is_short_hard: false,
                    suggested_tags,
                }
            }
            Err(_) => {
                tracing::debug!(raw = %trimmed, "Classifier returned non-JSON, using default");
                Classification::default()
            }
        }
    }

    #[allow(dead_code)]
    pub fn timeout_ms(&self) -> u64 {
        self.config.timeout_ms
    }
}
