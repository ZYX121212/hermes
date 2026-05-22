// crates/llm/src/adapter.rs
use async_trait::async_trait;
use futures::Stream;

/// Unified LLM adapter trait.
/// Supports completion, streaming completion, and text embedding.
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    /// Non-streaming completion: send a prompt, get back the full response.
    async fn complete(&self, prompt: String) -> anyhow::Result<String>;

    /// Streaming completion: returns a stream of text chunks.
    async fn complete_stream(
        &self,
        prompt: String,
    ) -> anyhow::Result<Box<dyn Stream<Item = anyhow::Result<String>> + Unpin + Send>>;

    /// Embed a text into a vector of floats.
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}
