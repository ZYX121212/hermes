// crates/llm/src/openai.rs
// Adapter for the OpenAI API (GPT-4, GPT-4o, etc.).
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::LlmAdapter;

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
        }
    }

    fn build_body(&self, prompt: String) -> Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": [{"role": "user", "content": prompt}]
        })
    }
}

#[async_trait]
impl LlmAdapter for OpenAIAdapter {
    async fn complete(&self, prompt: String) -> anyhow::Result<String> {
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&self.build_body(prompt))
            .send()
            .await?;

        let status = resp.status();
        let body_text = resp.text().await?;

        let body: Value = serde_json::from_str(&body_text).map_err(|e| {
            let preview = &body_text[..body_text.len().min(300)];
            anyhow::anyhow!(
                "API 返回了无效 JSON (HTTP {status})。\n原始响应: {preview}\n解析错误: {e}"
            )
        })?;

        if !status.is_success() {
            let err_msg = body["error"]["message"]
                .as_str()
                .unwrap_or("unknown error");
            return Err(anyhow::anyhow!("API 错误 ({}): {err_msg}", status));
        }

        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
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
            return Err(anyhow::anyhow!(
                "Stream error ({}): {err_body}",
                status
            ));
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
            tracing::debug!(
                "Embedding endpoint returned {status} — provider may not support embeddings, using zero vector"
            );
            return Ok(vec![0.0_f32; 1024]);
        }

        let body: Value = resp.json().await?;
        let embedding: Vec<f32> = body["data"][0]["embedding"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect()
            })
            .unwrap_or_default();

        Ok(embedding)
    }
}
