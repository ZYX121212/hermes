// crates/llm/src/anthropic.rs
// Adapter for the Anthropic (Claude) Messages API.
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{ChatCompletionRequest, LlmAdapter};
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
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("Failed to build reqwest client"),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            last_usage: Arc::new(Mutex::new(None)),
        }
    }

    /// Build the messages payload for the Anthropic Messages API (legacy flat prompt).
    fn build_body(&self, prompt: String) -> Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": [{"role": "user", "content": prompt}]
        })
    }

    /// Build structured messages payload for Anthropic Messages API.
    fn build_chat_body(&self, req: &ChatCompletionRequest) -> Value {
        let max_tokens = req.max_tokens.unwrap_or(self.max_tokens);

        let mut system_parts: Vec<String> = Vec::new();
        let mut messages: Vec<Value> = Vec::new();

        for msg in &req.messages {
            if msg.role == "system" {
                system_parts.push(msg.content.clone());
            } else {
                messages.push(serde_json::json!({
                    "role": msg.role,
                    "content": msg.content
                }));
            }
        }

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": messages,
        });

        if !system_parts.is_empty() {
            let system_text = system_parts.join("\n\n");
            body["system"] = serde_json::Value::String(system_text);
        }

        if let Some(temp) = req.temperature {
            body["temperature"] = serde_json::Value::Number(
                serde_json::Number::from_f64(temp).unwrap_or_else(|| serde_json::Number::from(0)),
            );
        }
        if let Some(top_p) = req.top_p {
            body["top_p"] = serde_json::Value::Number(
                serde_json::Number::from_f64(top_p).unwrap_or_else(|| serde_json::Number::from(0)),
            );
        }

        body
    }

    fn extract_usage(body: &Value) -> Option<TokenUsage> {
        let usage = body.get("usage")?;
        let input = usage.get("input_tokens")?.as_u64()?;
        let output = usage.get("output_tokens")?.as_u64()?;
        Some(TokenUsage::new(input, output, input + output))
    }

    /// Send a request to Anthropic with retry support.
    async fn do_request(&self, body: &Value) -> anyhow::Result<Value> {
        use crate::retry::with_retry;

        let payload = serde_json::to_string(body)?;
        with_retry("anthropic", 3, {
            let client = self.client.clone();
            let api_key = self.api_key.clone();
            let payload = payload.clone();
            move || {
                let client = client.clone();
                let api_key = api_key.clone();
                let payload = payload.clone();
                async move {
                    let resp = client
                        .post("https://api.anthropic.com/v1/messages")
                        .header("x-api-key", &api_key)
                        .header("anthropic-version", "2023-06-01")
                        .header("content-type", "application/json")
                        .body(payload)
                        .send()
                        .await?;

                    let status = resp.status();
                    let body_json: Value = resp.json().await?;

                    if !status.is_success() {
                        let err_msg = body_json["error"]["message"]
                            .as_str()
                            .unwrap_or("unknown error");
                        return Err(anyhow::anyhow!(
                            "Anthropic API error ({}): {err_msg}",
                            status
                        ));
                    }

                    Ok(body_json)
                }
            }
        })
        .await
    }

    fn extract_completion_text(&self, body: &Value) -> String {
        body["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| {
                tracing::error!(
                    body = %serde_json::to_string_pretty(body).unwrap_or_else(|_| "(unprintable)".into()),
                    "Unexpected Anthropic API response structure"
                );
                ""
            })
            .to_string()
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

        let body = self.build_body(prompt);
        let body_json = self.do_request(&body).await?;

        if let Some(usage) = Self::extract_usage(&body_json) {
            tracing::debug!(
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                "Anthropic token usage"
            );
            *self.last_usage.lock() = Some(usage);
        }

        Ok(self.extract_completion_text(&body_json))
    }

    async fn complete_chat(&self, req: ChatCompletionRequest) -> anyhow::Result<String> {
        tracing::info!(
            provider = "anthropic",
            model = %self.model,
            msg_count = req.messages.len(),
            "LLM chat completion request"
        );

        let body = self.build_chat_body(&req);
        let body_json = self.do_request(&body).await?;

        if let Some(usage) = Self::extract_usage(&body_json) {
            tracing::debug!(
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                "Anthropic token usage"
            );
            *self.last_usage.lock() = Some(usage);
        }

        Ok(self.extract_completion_text(&body_json))
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

    async fn complete_stream_chat(
        &self,
        req: ChatCompletionRequest,
    ) -> anyhow::Result<Box<dyn Stream<Item = anyhow::Result<String>> + Unpin + Send>> {
        let mut body = self.build_chat_body(&req);
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
