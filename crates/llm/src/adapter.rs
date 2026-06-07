// crates/llm/src/adapter.rs
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::usage::TokenUsage;

/// Gateway routing metadata extracted from response headers.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub routed_model: String,
    pub shg_triggered: bool,
    pub reason: String,
}

/// A single chat message with role and content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Structured chat completion request with full parameter support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stream: bool,
}

impl ChatCompletionRequest {
    /// Flatten messages into a single prompt string for legacy adapters.
    pub fn flatten(&self) -> String {
        self.messages
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 统一的 LLM 适配器 trait。
/// 支持 completion、流式 completion 和文本嵌入。
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    /// 非流式 completion：发送 prompt，返回完整响应。
    async fn complete(&self, prompt: String) -> anyhow::Result<String>;

    /// 流式 completion：返回文本块流。
    async fn complete_stream(
        &self,
        prompt: String,
    ) -> anyhow::Result<Box<dyn Stream<Item = anyhow::Result<String>> + Unpin + Send>>;

    /// 非流式 structured chat completion：透传完整消息结构和参数。
    /// Default implementation falls back to `complete()` with flattened prompt.
    async fn complete_chat(&self, req: ChatCompletionRequest) -> anyhow::Result<String> {
        self.complete(req.flatten()).await
    }

    /// 流式 structured chat completion：透传完整消息结构和参数。
    /// Default implementation falls back to `complete_stream()` with flattened prompt.
    async fn complete_stream_chat(
        &self,
        req: ChatCompletionRequest,
    ) -> anyhow::Result<Box<dyn Stream<Item = anyhow::Result<String>> + Unpin + Send>> {
        self.complete_stream(req.flatten()).await
    }

    /// 将文本嵌入为浮点向量。
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;

    /// 获取最近一次调用的 token 用量（默认返回 None）。
    fn last_usage(&self) -> Option<TokenUsage> {
        None
    }

    /// 获取最近一次调用的 gateway 路由信息（默认返回 None）。
    fn last_route_info(&self) -> Option<RouteInfo> {
        None
    }

    /// 获取共享的 route mode 引用，供 TUI 更新路由模式（默认返回 None）。
    fn route_mode(&self) -> Option<Arc<Mutex<Option<String>>>> {
        None
    }
}
