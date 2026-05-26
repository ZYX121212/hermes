// crates/llm/src/anthropic.rs
// Adapter for the Anthropic (Claude) Messages API.
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::LlmAdapter;
use crate::usage::TokenUsage;

/// Configuration for an Anthropic API connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

/// Adapter for the Anthropic (Claude) API.
pub struct AnthropicAdapter {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    last_usage: Arc<Mutex<Option<TokenUsage>>>,
}

impl AnthropicAdapter {
    /// Create a new Anthropic adapter from configuration.
    pub fn new(cfg: &AnthropicConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            last_usage: Arc::new(Mutex::new(None)),
        }
    }

    /// Build the messages payload for the Anthropic Messages API.
    fn build_body(&self, prompt: String) -> Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": [{"role": "user", "content": prompt}]
        })
    }

    fn extract_usage(body: &Value) -> Option<TokenUsage> {
        let usage = body.get("usage")?;
        let input = usage.get("input_tokens")?.as_u64()?;
        let output = usage.get("output_tokens")?.as_u64()?;
        Some(TokenUsage::new(input, output, input + output))
    }
}

#[async_trait]
impl LlmAdapter for AnthropicAdapter {
    async fn complete(&self, prompt: String) -> anyhow::Result<String> {
        tracing::info!(
            provider = "anthropic",
            model = %self.model,
            prompt_len = prompt.len(),
            "LLM completion request"
        );

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&self.build_body(prompt))
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp.json().await?;

        if !status.is_success() {
            let err_msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            return Err(anyhow::anyhow!(
                "Anthropic API error ({}): {err_msg}",
                status
            ));
        }

        // Extract usage
        if let Some(usage) = Self::extract_usage(&body) {
            tracing::debug!(
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                "Anthropic token usage"
            );
            *self.last_usage.lock() = Some(usage);
        }

        let text = body["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| {
                tracing::error!(
                    body = %serde_json::to_string_pretty(&body).unwrap_or_else(|_| "(unprintable)".into()),
                    "Unexpected Anthropic API response structure"
                );
                ""
            })
            .to_string();

        Ok(text)
    }

    async fn complete_stream(
        &self,
        prompt: String,
    ) -> anyhow::Result<Box<dyn Stream<Item = anyhow::Result<String>> + Unpin + Send>> {
        let mut body = self.build_body(prompt);
        body["stream"] = serde_json::Value::Bool(true);

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Anthropic stream error ({}): {err_body}",
                status
            ));
        }

        use crate::stream::SseChunkStream;
        Ok(Box::new(SseChunkStream::new(Box::pin(resp.bytes_stream()))))
    }

    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        use std::sync::atomic::{AtomicBool, Ordering};
        static EMBED_WARNED: AtomicBool = AtomicBool::new(false);
        if !EMBED_WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "AnthropicAdapter: embed() returning zero vector (use VoyageEmbedder instead)"
            );
        }
        Ok(vec![0.0_f32; 1024])
    }

    fn last_usage(&self) -> Option<TokenUsage> {
        self.last_usage.lock().clone()
    }
}
