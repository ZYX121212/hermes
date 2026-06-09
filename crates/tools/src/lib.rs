//! Extensible tool framework with built-in bash, file, browser, search, and plugin support.
//!

pub mod browser;
pub mod builtin;
pub mod caller;
pub mod code_exec;
pub mod mcp_tools;
pub mod plugin;
pub mod registry;
pub mod search;
pub mod toolset;
pub mod vision;

pub use browser::{
    browser_toolset, BrowserClickTool, BrowserExecuteTool, BrowserFillTool,
    BrowserNavigateTool, BrowserScreenshotTool,
};
pub use builtin::{
    ApprovalPolicy, ApprovalResult, BashTool, ConfirmationPolicy, DangerGuard, ReadFileTool,
    ReplyTool, SearchConfig, ToolGuard, WebSearchTool, WriteFileTool,
};
pub use code_exec::{CodeExecBashTool, ExecMode, JsExecTool, PythonExecTool, SandboxConfig};
pub use mcp_tools::{McpCallToolTool, McpToolListTool, McpToolProxy};
pub use plugin::{discover_plugins, PluginManifest, ScriptPlugin, ShellPlugin};
pub use registry::ToolRegistry;
pub use search::{
    FetchConfig, ImageSearchTool, NewsSearchTool, SerperClient, TavilyClient, WebFetchTool,
};
pub use toolset::{ToolsetKind, ToolsetRegistry};
pub use vision::{VisionDescribeTool, VisionTool};

use async_trait::async_trait;
use futures::stream::Stream;

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

/// 流式工具输出块。
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// 文本增量
    Text(String),
    /// 流结束，包含完整结果
    Done(ToolOutput),
    /// 流错误
    Error(String),
}

/// The Tool trait: every tool the agent can call implements this.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;

    /// 执行工具并返回结果。
    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput>;

    /// 流式执行工具，返回增量数据流。
    /// 默认实现回退到 call() 并发送单个 Done 块。
    fn call_stream(
        &self,
        args: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn Stream<Item = StreamChunk> + Send + '_>> {
        let this = self;
        Box::pin(async_stream::stream! {
            match this.call(args).await {
                Ok(output) => yield StreamChunk::Done(output),
                Err(e) => yield StreamChunk::Error(format!("{e:#}")),
            }
        })
    }
}

/// 包装一个 Stream<Item = String> 为工具流式输出的工具辅助函数。
pub fn stream_to_chunks(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<StreamChunk>,
) -> impl Stream<Item = StreamChunk> {
    async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield chunk;
        }
    }
}
