// crates/llm/src/openai.rs
// Adapter for the OpenAI API (GPT-4, GPT-4o, etc.).
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{LlmAdapter, RouteInfo};
use crate::usage::TokenUsage;

/// Configuration for an OpenAI-compatible API connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    #[serde(default = "default_base_url")]
    pub base_url: String,
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".into()
}

/// Adapter for OpenAI-compatible APIs (OpenAI, DeepSeek, Groq, etc.).
pub struct OpenAIAdapter {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    base_url: String,
    last_usage: Arc<Mutex<Option<TokenUsage>>>,
    last_route_info: Arc<Mutex<Option<RouteInfo>>>,
    /// Shared route mode, updated by TUI settings panel.
    pub route_mode: Arc<Mutex<Option<String>>>,
}

impl OpenAIAdapter {
    /// Create a new OpenAI-compatible adapter from configuration.
    pub fn new(cfg: &OpenAIConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            base_url: cfg.base_url.clone(),
            last_usage: Arc::new(Mutex::new(None)),
            last_route_info: Arc::new(Mutex::new(None)),
            route_mode: Arc::new(Mutex::new(None)),
        }
    }

    fn build_body(&self, prompt: String) -> Value {
        let mode = self.route_mode.lock().clone();
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": [{"role": "user", "content": prompt}]
        });
        if let Some(ref m) = mode {
            body["mode"] = serde_json::Value::String(m.clone());
        }
        body
    }

    fn extract_usage(body: &Value) -> Option<TokenUsage> {
        let usage = body.get("usage")?;
        Some(TokenUsage::new(
            usage.get("prompt_tokens")?.as_u64()?,
            usage.get("completion_tokens")?.as_u64()?,
            usage.get("total_tokens")?.as_u64()?,
        ))
    }
}

#[async_trait]
impl LlmAdapter for OpenAIAdapter {
    async fn complete(&self, prompt: String) -> anyhow::Result<String> {
        tracing::info!(
            provider = "openai",
            model = %self.model,
            prompt_len = prompt.len(),
            "LLM completion request"
        );

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&self.build_body(prompt))
            .send()
            .await?;

        // Capture gateway routing headers before consuming body
        let route_info = {
            let headers = resp.headers();
            let routed = headers
                .get("x-hermess-routed-model")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            if let Some(model) = routed {
                let shg = headers
                    .get("x-hermess-shg-triggered")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s == "true")
                    .unwrap_or(false);
                let reason = headers
                    .get("x-hermess-route-reason")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                Some(RouteInfo {
                    routed_model: model,
                    shg_triggered: shg,
                    reason,
                })
            } else {
                None
            }
        };

        let status = resp.status();
        let body_text = resp.text().await?;

        let body: Value = serde_json::from_str(&body_text).map_err(|e| {
            let preview = &body_text[..body_text.len().min(300)];
            anyhow::anyhow!(
                "API 返回了无效 JSON (HTTP {status})。\n原始响应: {preview}\n解析错误: {e}"
            )
        })?;

        if !status.is_success() {
            let err_msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            return Err(anyhow::anyhow!("API 错误 ({}): {err_msg}", status));
        }

        // Extract usage
        if let Some(usage) = Self::extract_usage(&body) {
            tracing::debug!(
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                "OpenAI token usage"
            );
            *self.last_usage.lock() = Some(usage);
        }

        // Store gateway route info
        if route_info.is_some() {
            *self.last_route_info.lock() = route_info;
        }

        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_else(|| {
                tracing::error!(
                    body = %serde_json::to_string_pretty(&body).unwrap_or_else(|_| "(unprintable)".into()),
                    "Unexpected OpenAI API response structure"
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
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Stream error ({}): {err_body}", status));
        }

        use crate::stream::SseChunkStream;
        Ok(Box::new(SseChunkStream::new(Box::pin(resp.bytes_stream()))))
    }

    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let resp = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": "text-embedding-3-small",
                "input": text
            }))
            .send()
            .await?;

        let status = resp.status();
        // DeepSeek and some other providers don't support embeddings.
        // Return a zero vector gracefully instead of failing.
        if !status.is_success() {
            use std::sync::atomic::{AtomicBool, Ordering};
            static EMBED_WARNED: AtomicBool = AtomicBool::new(false);
            if !EMBED_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    status = %status,
                    "Embedding endpoint returned {status} — provider may not support embeddings, using zero vector"
                );
            }
            return Ok(vec![0.0_f32; 1024]);
        }

        let body: Value = resp.json().await?;
        let embedding: Vec<f32> = body["data"][0]["embedding"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|v| {
                        v.as_f64().map(|x| x as f32).unwrap_or_else(|| {
                            tracing::warn!(value = %v, "Non-numeric embedding value");
                            0.0
                        })
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                tracing::warn!("Embedding response data array is empty or missing");
                Vec::new()
            });

        Ok(embedding)
    }

    fn last_usage(&self) -> Option<TokenUsage> {
        self.last_usage.lock().clone()
    }

    fn last_route_info(&self) -> Option<RouteInfo> {
        self.last_route_info.lock().clone()
    }

    fn route_mode(&self) -> Option<Arc<Mutex<Option<String>>>> {
        Some(Arc::clone(&self.route_mode))
    }
}
