// crates/tools/src/lib.rs
pub mod builtin;
pub mod caller;
pub mod registry;

pub use builtin::{BashTool, ReadFileTool, SearchConfig, WebSearchTool, WriteFileTool};
pub use registry::ToolRegistry;

use async_trait::async_trait;

/// Output of a tool invocation.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub success: bool,
    pub content: String,
    pub metadata: serde_json::Value,
}

impl ToolOutput {
    pub fn text(content: String) -> Self {
        Self {
            success: true,
            content,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn error(content: String) -> Self {
        Self {
            success: false,
            content,
            metadata: serde_json::Value::Null,
        }
    }
}

/// The Tool trait: every tool the agent can call implements this.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput>;
}
