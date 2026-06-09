//! MCP (Model Context Protocol) implementation for Hermes Agent.
pub mod client;
pub mod protocol;
pub mod server;

pub use client::{McpClient, McpTransport};
pub use protocol::{
    JsonRpcRequest, JsonRpcResponse, ServerCapabilities, ToolCallContent, ToolCallParams,
    ToolCallResult, ToolDef,
};
pub use server::{run_stdio_server, McpHandler};
