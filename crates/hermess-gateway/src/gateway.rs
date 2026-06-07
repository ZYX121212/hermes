use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::classifier::ComplexityClassifier;
use crate::config::GatewayConfig;
use crate::decision::DecisionEngine;
use crate::decomposer::PromptDecomposer;
use crate::discovery;

use crate::feedback::FeedbackTracker;
use crate::metrics::RouteMetrics;
use crate::models::{ChatMessage, GatewayOutput, RouteMode};
use crate::registry::ModelRegistry;
use crate::shg::ShgDetector;
use crate::skills::SkillSet;

/// Core gateway orchestrator. Owns all layers and exposes a single `route` method.
pub struct Gateway {
    pub config: GatewayConfig,
    pub instance_name: String,
    registry: Arc<ModelRegistry>,
    decision_engine: DecisionEngine,
    decomposer: Option<PromptDecomposer>,
    pub session_history: Mutex<Vec<(String, String)>>,
    pub metrics: Arc<RouteMetrics>,
    pub feedback: Arc<FeedbackTracker>,
    pub started_at: Instant,
    pub classifier_model: String,
    pub skill_set: SkillSet,
}

impl Gateway {
    pub async fn new(config: GatewayConfig, instance_name: &str, fresh: bool) -> Self {
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

        // Discover project skills from .hermess/skills/
        let skill_set = SkillSet::discover();

        // SHG with patterns merged from skills
        let mut shg = ShgDetector::new(&config.gateway.shg);
        let skill_patterns = skill_set.shg_patterns();
        if !skill_patterns.is_empty() {
            shg.merge_patterns(&skill_patterns);
            tracing::info!(
                count = skill_patterns.len(),
                "Merged SHG patterns from skills"
            );
        }

        let decomposer = if config.gateway.optimizer.decompose_enabled {
            Some(PromptDecomposer::new(
                config.gateway.classifier.model.clone(),
                (*registry).clone(),
            ))
        } else {
            None
        };

        let feedback_file = format!(".hermes_feedback_{instance_name}.json");
        let metrics = Arc::new(RouteMetrics::new());
        let feedback = if fresh {
            tracing::info!(instance = %instance_name, "Starting with fresh memory");
            Arc::new(FeedbackTracker::new())
        } else {
            Arc::new(FeedbackTracker::load_from_file(&feedback_file))
        };

        let domain_ctx = skill_set.domain_context();
        let classifier =
            ComplexityClassifier::new(config.gateway.classifier.clone(), Arc::clone(&registry))
                .with_metrics(Arc::clone(&metrics))
                .with_domain_context(domain_ctx);

        let decision_engine = DecisionEngine::new(
            shg,
            classifier,
            config.gateway.optimizer.clone(),
            Arc::clone(&registry),
            Arc::clone(&feedback),
        );

        let classifier_model = config.gateway.classifier.model.clone();

        Self {
            config,
            instance_name: instance_name.to_string(),
            registry,
            decision_engine,
            decomposer,
            session_history: Mutex::new(Vec::new()),
            metrics,
            feedback,
            started_at: Instant::now(),
            classifier_model,
            skill_set,
        }
    }

    /// Route a chat completion request. Returns (output, reasoning).
    pub async fn route(&self, prompt: &str, mode: Option<RouteMode>) -> (GatewayOutput, String) {
        let mode = mode
            .or_else(|| self.skill_set.route_hint())
            .unwrap_or_else(|| self.config.gateway.default_mode.clone());
        let decision = self.decision_engine.decide(prompt, &mode).await;
        let reasoning = decision.reasoning;

        let output = match decision.target {
            crate::models::RouteTarget::Single(model) => GatewayOutput::Single {
                model,
                prompt: prompt.to_string(),
            },
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

    /// Record a routing interaction to short-term session history.
    pub async fn record_interaction(&self, prompt_preview: &str, model: &str) {
        let mut hist = self.session_history.lock().await;
        hist.push((prompt_preview.to_string(), model.to_string()));
        if hist.len() > 100 {
            hist.remove(0);
        }
    }

    /// Extract a concise text representation of the conversation for the routing pipeline.
    /// System messages come first; content is truncated to avoid overloading the classifier.
    pub fn extract_prompt(messages: &[ChatMessage]) -> String {
        if messages.is_empty() {
            return String::new();
        }
        // Separate system messages from the rest
        let (system_msgs, other_msgs): (Vec<_>, Vec<_>) =
            messages.iter().partition(|m| m.role == "system");

        let mut parts: Vec<String> = Vec::new();

        // System context first — most important for routing decisions
        for m in &system_msgs {
            let content = Self::truncate_content(&m.content, 500);
            parts.push(format!("[system]: {content}"));
        }

        // Then the actual conversation
        for m in &other_msgs {
            let content = Self::truncate_content(&m.content, 1000);
            parts.push(format!("[{}]: {content}", m.role));
        }

        // Cap total length for classifier efficiency
        let joined = parts.join("\n");
        if joined.len() > 4000 {
            let cutoff = joined
                .char_indices()
                .take(4000)
                .last()
                .map(|(i, _)| i)
                .unwrap_or(4000);
            joined[..cutoff].to_string()
        } else {
            joined
        }
    }

    fn truncate_content(content: &str, max_chars: usize) -> &str {
        if content.len() <= max_chars {
            return content;
        }
        let cutoff = content
            .char_indices()
            .take(max_chars)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(max_chars);
        &content[..cutoff]
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
            "anthropic" => Some(Box::new(llm::AnthropicAdapter::new(
                &llm::AnthropicConfig {
                    api_key: entry.api_key.clone(),
                    model: entry.name.clone(),
                    max_tokens: 4096,
                },
            ))),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_prompt_empty() {
        assert_eq!(Gateway::extract_prompt(&[]), "");
    }

    #[test]
    fn extract_prompt_basic() {
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: "hello".into(),
        }];
        let result = Gateway::extract_prompt(&msgs);
        assert!(result.contains("hello"));
    }

    #[test]
    fn extract_prompt_system_first() {
        let msgs = vec![
            ChatMessage {
                role: "user".into(),
                content: "question".into(),
            },
            ChatMessage {
                role: "system".into(),
                content: "context".into(),
            },
        ];
        let result = Gateway::extract_prompt(&msgs);
        // System message should appear first regardless of input order
        let sys_pos = result.find("[system]").unwrap();
        let user_pos = result.find("[user]").unwrap();
        assert!(
            sys_pos < user_pos,
            "system should come before user, got: {result}"
        );
    }

    #[test]
    fn extract_prompt_truncates_long_content() {
        let long = "a".repeat(600);
        let msgs = vec![ChatMessage {
            role: "system".into(),
            content: long,
        }];
        let result = Gateway::extract_prompt(&msgs);
        // System messages truncated to 500 chars
        let content_only = result.strip_prefix("[system]: ").unwrap();
        assert!(content_only.len() <= 510); // 500 chars + some margin
    }

    #[test]
    fn extract_prompt_caps_total_length() {
        let long = "x".repeat(1000);
        let mut msgs = Vec::new();
        for _ in 0..10 {
            msgs.push(ChatMessage {
                role: "user".into(),
                content: long.clone(),
            });
        }
        let result = Gateway::extract_prompt(&msgs);
        assert!(
            result.len() <= 4100,
            "total length capped at 4000, got {}",
            result.len()
        );
    }
}
