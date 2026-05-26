use std::sync::Arc;
use tokio::sync::Mutex;

use crate::classifier::ComplexityClassifier;
use crate::config::GatewayConfig;
use crate::decision::DecisionEngine;
use crate::decomposer::PromptDecomposer;
use crate::discovery;
use crate::distiller::ContextDistiller;
use crate::models::{ChatMessage, GatewayOutput, RouteMode};
use crate::registry::ModelRegistry;
use crate::shg::ShgDetector;

/// Core gateway orchestrator. Owns all layers and exposes a single `route` method.
pub struct Gateway {
    pub config: GatewayConfig,
    registry: Arc<ModelRegistry>,
    decision_engine: DecisionEngine,
    decomposer: Option<PromptDecomposer>,
    #[allow(dead_code)]
    distiller: Option<ContextDistiller>,
    #[allow(dead_code)]
    pub session_history: Mutex<Vec<(String, String)>>,
}

impl Gateway {
    pub async fn new(config: GatewayConfig) -> Self {
        let mut registry = ModelRegistry::from_entries(config.gateway.models.clone());

        // Auto-discover models from configured upstream providers
        for provider in &config.gateway.discovery {
            let discovered = discovery::discover(provider).await;
            for entry in discovered {
                // User-configured models take priority over discovered ones
                if registry.get(&entry.name).is_none() {
                    registry.add(entry);
                }
            }
        }

        let registry = Arc::new(registry);

        let shg = ShgDetector::new(&config.gateway.shg);
        let classifier = ComplexityClassifier::new(
            config.gateway.classifier.clone(),
            Arc::clone(&registry),
        );

        let decision_engine = DecisionEngine::new(
            shg,
            classifier,
            config.gateway.optimizer.clone(),
            Arc::clone(&registry),
        );

        let decomposer = if config.gateway.optimizer.decompose_enabled {
            Some(PromptDecomposer::new(
                config.gateway.classifier.model.clone(),
                (*registry).clone(),
            ))
        } else {
            None
        };

        let distiller = if config.gateway.optimizer.distill_enabled {
            Some(ContextDistiller::new(config.gateway.optimizer.distill_keep_ratio))
        } else {
            None
        };

        Self {
            config,
            registry,
            decision_engine,
            decomposer,
            distiller,
            session_history: Mutex::new(Vec::new()),
        }
    }

    /// Route a chat completion request. Returns (output, reasoning).
    pub async fn route(
        &self,
        prompt: &str,
        mode: Option<RouteMode>,
    ) -> (GatewayOutput, String) {
        let mode = mode.unwrap_or_else(|| self.config.gateway.default_mode.clone());
        let decision = self.decision_engine.decide(prompt, &mode).await;
        let reasoning = decision.reasoning;

        let output = match decision.target {
            crate::models::RouteTarget::Single(model) => {
                GatewayOutput::Single {
                    model,
                    prompt: prompt.to_string(),
                }
            }
            crate::models::RouteTarget::Decomposed { critical, regular } => {
                let decomposed = if let Some(ref dec) = self.decomposer {
                    dec.decompose(prompt).await
                } else {
                    crate::models::DecomposedPrompt {
                        critical: prompt.to_string(),
                        regular: String::new(),
                    }
                };
                GatewayOutput::Decomposed {
                    critical_model: critical,
                    critical_prompt: decomposed.critical,
                    regular_model: regular,
                    regular_prompt: decomposed.regular,
                }
            }
        };

        (output, reasoning)
    }

    /// Extract the concatenated user prompt from OpenAI messages.
    pub fn extract_prompt(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Build an LLM adapter from a registry entry.
    pub fn build_adapter(entry: &crate::models::ModelEntry) -> Option<Box<dyn llm::LlmAdapter>> {
        match entry.provider.as_str() {
            "openai" | "deepseek" => Some(Box::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: entry.api_key.clone(),
                model: entry.name.clone(),
                max_tokens: 4096,
                base_url: if entry.base_url.is_empty() {
                    "https://api.openai.com/v1".into()
                } else {
                    entry.base_url.clone()
                },
            }))),
            "anthropic" => Some(Box::new(llm::AnthropicAdapter::new(&llm::AnthropicConfig {
                api_key: entry.api_key.clone(),
                model: entry.name.clone(),
                max_tokens: 4096,
            }))),
            _ => {
                tracing::warn!(provider = %entry.provider, "Unknown provider");
                None
            }
        }
    }

    pub fn lookup_model(&self, name: &str) -> Option<crate::models::ModelEntry> {
        self.registry.get(name).cloned()
    }

    pub fn list_models(&self) -> &[crate::models::ModelEntry] {
        self.registry.all()
    }

    #[allow(dead_code)]
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }
}
