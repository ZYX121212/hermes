// crates/llm/src/lib.rs
pub mod adapter;
pub mod anthropic;
pub mod openai;
pub mod stream;

pub use adapter::LlmAdapter;
pub use anthropic::{AnthropicAdapter, AnthropicConfig};
pub use openai::{OpenAIAdapter, OpenAIConfig};
