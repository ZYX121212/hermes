use crate::models::DecomposedPrompt;
use crate::registry::ModelRegistry;
use llm::LlmAdapter;

/// Splits a prompt into critical and regular parts using a lightweight LLM.
pub struct PromptDecomposer {
    model_name: String,
    registry: ModelRegistry,
}

impl PromptDecomposer {
    pub fn new(model_name: String, registry: ModelRegistry) -> Self {
        Self {
            model_name,
            registry,
        }
    }

    /// Attempt to split the prompt. Falls back to a simple heuristic
    /// (entire prompt -> critical, empty regular) on any error.
    pub async fn decompose(&self, prompt: &str) -> DecomposedPrompt {
        let model = match self.registry.get(&self.model_name) {
            Some(m) => m,
            None => {
                tracing::warn!("Decomposer model not found, using fallback");
                return self.fallback(prompt);
            }
        };

        let split_prompt = format!(
            "Split the following request into two parts:\n\
             - CRITICAL: the core task requiring deep reasoning or precise knowledge\n\
             - REGULAR: routine context, formatting, or simple follow-up actions\n\n\
             Respond with ONLY valid JSON: {{\"critical\": \"...\", \"regular\": \"...\"}}\n\n\
             Request: {prompt}"
        );

        let adapter: Box<dyn LlmAdapter> = match model.provider.as_str() {
            "openai" | "deepseek" => Box::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: model.api_key.clone(),
                model: model.name.clone(),
                max_tokens: 256,
                base_url: if model.base_url.is_empty() {
                    "https://api.openai.com/v1".into()
                } else {
                    model.base_url.clone()
                },
            })),
            _ => return self.fallback(prompt),
        };

        match adapter.complete(split_prompt).await {
            Ok(response) => {
                let trimmed = response
                    .trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();
                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(val) => DecomposedPrompt {
                        critical: val["critical"].as_str().unwrap_or(prompt).to_string(),
                        regular: val["regular"].as_str().unwrap_or("").to_string(),
                    },
                    Err(_) => self.fallback(prompt),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Decomposer LLM call failed, using fallback");
                self.fallback(prompt)
            }
        }
    }

    fn fallback(&self, prompt: &str) -> DecomposedPrompt {
        DecomposedPrompt {
            critical: prompt.to_string(),
            regular: String::new(),
        }
    }
}
