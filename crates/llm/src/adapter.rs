// crates/llm/src/adapter.rs
use async_trait::async_trait;
use futures::Stream;

use crate::usage::TokenUsage;

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

    /// 将文本嵌入为浮点向量。
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;

    /// 获取最近一次调用的 token 用量（默认返回 None）。
    fn last_usage(&self) -> Option<TokenUsage> {
        None
    }
}
