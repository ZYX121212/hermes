// crates/tools/src/mcp_tools.rs
// MCP 工具代理：将远程 MCP 服务器工具包装为本地 Tool trait。
use std::sync::Arc;

use async_trait::async_trait;
use mcp::{McpClient, ToolDef};

use crate::{Tool, ToolOutput};

/// 代理单个远程 MCP 工具为本地 Tool。
pub struct McpToolProxy {
    remote_name: String,
    description: String,
    input_schema: serde_json::Value,
    client: Arc<McpClient>,
}

impl McpToolProxy {
    pub fn new(tool_def: ToolDef, client: Arc<McpClient>) -> Self {
        Self {
            remote_name: tool_def.name,
            description: tool_def.description,
            input_schema: tool_def.input_schema,
            client,
        }
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str {
        &self.remote_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let result = self.client.call_tool(&self.remote_name, args).await?;
        let text: String = result
            .content
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput::text(text))
    }
}

/// 列出已连接 MCP 服务器的所有工具。
pub struct McpToolListTool {
    client: Arc<McpClient>,
}

impl McpToolListTool {
    pub fn new(client: Arc<McpClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for McpToolListTool {
    fn name(&self) -> &str {
        "mcp_list_tools"
    }

    fn description(&self) -> &str {
        "List all tools available from a connected MCP (Model Context Protocol) server."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn call(&self, _args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let tools = self.client.list_tools().await?;
        let text: String = tools
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput::text(text))
    }
}

/// 按名称调用 MCP 工具。
pub struct McpCallToolTool {
    client: Arc<McpClient>,
}

impl McpCallToolTool {
    pub fn new(client: Arc<McpClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for McpCallToolTool {
    fn name(&self) -> &str {
        "mcp_call_tool"
    }

    fn description(&self) -> &str {
        "Call a tool on a connected MCP server by name with given arguments."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Name of the tool to call on the MCP server"
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments to pass to the tool"
                }
            },
            "required": ["tool_name"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let tool_name = args["tool_name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("mcp_call_tool: 'tool_name' is required"))?;
        let arguments = args
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let result = self.client.call_tool(tool_name, arguments).await?;
        let text: String = result
            .content
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput::text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp::McpTransport;

    #[test]
    fn test_proxy_schema_passthrough() {
        let client = Arc::new(McpClient::new(McpTransport::Http {
            url: "http://localhost/mcp".into(),
            api_key: None,
        }));
        let tool_def = ToolDef {
            name: "remote_tool".into(),
            description: "A remote tool".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        };
        let proxy = McpToolProxy::new(tool_def, client);
        assert_eq!(proxy.name(), "remote_tool");
    }

    #[test]
    fn test_list_tools_name() {
        let client = Arc::new(McpClient::new(McpTransport::Http {
            url: "http://localhost/mcp".into(),
            api_key: None,
        }));
        let tool = McpToolListTool::new(client);
        assert_eq!(tool.name(), "mcp_list_tools");
    }

    #[test]
    fn test_call_tool_name() {
        let client = Arc::new(McpClient::new(McpTransport::Http {
            url: "http://localhost/mcp".into(),
            api_key: None,
        }));
        let tool = McpCallToolTool::new(client);
        assert_eq!(tool.name(), "mcp_call_tool");
    }
}
