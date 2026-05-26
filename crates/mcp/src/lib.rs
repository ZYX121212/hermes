// crates/mcp/src/lib.rs
// MCP (Model Context Protocol) implementation for Hermes Agent.
pub mod protocol;
pub mod server;

pub use protocol::{
    JsonRpcRequest, JsonRpcResponse, ServerCapabilities, ToolCallContent, ToolCallParams,
    ToolCallResult, ToolDef,
};
pub use server::{run_stdio_server, McpHandler};
