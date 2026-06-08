// crates/mcp/src/client.rs
// MCP 客户端：连接外部 MCP 服务器（stdio/HTTP/SSE），发现工具并代理调用。
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::protocol::*;

/// MCP 传输类型
#[derive(Debug, Clone)]
pub enum McpTransport {
    /// 启动子进程，通过 stdin/stdout 通信
    Stdio { command: String, args: Vec<String> },
    /// HTTP endpoint（POST JSON-RPC）
    Http { url: String, api_key: Option<String> },
    /// SSE endpoint
    Sse { url: String, api_key: Option<String> },
}

/// MCP 客户端：连接 MCP 服务器并提供工具发现/调用。
pub struct McpClient {
    transport: McpTransport,
    http_client: reqwest::Client,
    server_info: Mutex<Option<serde_json::Value>>,
    child: Mutex<Option<Child>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl McpClient {
    pub fn new(transport: McpTransport) -> Self {
        Self {
            transport,
            http_client: reqwest::Client::new(),
            server_info: Mutex::new(None),
            child: Mutex::new(None),
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> serde_json::Value {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        serde_json::Value::Number(id.into())
    }

    async fn http_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let url = match &self.transport {
            McpTransport::Http { url, .. } => url.clone(),
            McpTransport::Sse { url, .. } => url.clone(),
            _ => anyhow::bail!("HTTP transport not configured"),
        };

        let req_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id(),
            "method": method,
            "params": params,
        });

        let mut req = self.http_client.post(&url).json(&req_body);

        let api_key = match &self.transport {
            McpTransport::Http { api_key, .. } | McpTransport::Sse { api_key, .. } => api_key,
            _ => &None,
        };
        if let Some(key) = api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req.send().await?;
        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    async fn stdio_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let (command, args) = match &self.transport {
            McpTransport::Stdio { command, args } => (command.clone(), args.clone()),
            _ => anyhow::bail!("Stdio transport not configured"),
        };

        let mut child = Command::new(&command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);

        let req = JsonRpcRequest::new(serde_json::Value::Number(1.into()), method, params);
        let req_str = format!("{}\n", serde_json::to_string(&req)?);
        stdin.write_all(req_str.as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin);

        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let json: serde_json::Value = serde_json::from_str(&line)?;

        child.kill().await?;
        Ok(json)
    }

    /// 初始化连接并握手。
    pub async fn initialize(&self) -> anyhow::Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "Hermes", "version": "0.1.0" }
        });

        let result = match &self.transport {
            McpTransport::Http { .. } | McpTransport::Sse { .. } => {
                self.http_call(METHOD_INITIALIZE, params).await?
            }
            McpTransport::Stdio { .. } => {
                self.stdio_call(METHOD_INITIALIZE, params).await?
            }
        };

        let mut info = self.server_info.lock().await;
        *info = Some(result);
        tracing::info!("MCP client initialized");
        Ok(())
    }

    /// 列出远程服务器的所有工具。
    pub async fn list_tools(&self) -> anyhow::Result<Vec<ToolDef>> {
        let result = match &self.transport {
            McpTransport::Http { .. } | McpTransport::Sse { .. } => {
                self.http_call(METHOD_TOOLS_LIST, serde_json::json!({})).await?
            }
            McpTransport::Stdio { .. } => {
                self.stdio_call(METHOD_TOOLS_LIST, serde_json::json!({})).await?
            }
        };

        let tools: Vec<ToolDef> = serde_json::from_value(result["result"]["tools"].clone())
            .unwrap_or_default();
        Ok(tools)
    }

    /// 调用远程工具。
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<ToolCallResult> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result = match &self.transport {
            McpTransport::Http { .. } | McpTransport::Sse { .. } => {
                self.http_call(METHOD_TOOLS_CALL, params).await?
            }
            McpTransport::Stdio { .. } => {
                self.stdio_call(METHOD_TOOLS_CALL, params).await?
            }
        };

        let call_result: ToolCallResult =
            serde_json::from_value(result["result"].clone())?;
        Ok(call_result)
    }

    /// 断开连接。
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        if let Some(mut c) = self.child.lock().await.take() {
            c.kill().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation_http() {
        let client = McpClient::new(McpTransport::Http {
            url: "http://localhost:8080/mcp".into(),
            api_key: None,
        });
        assert!(client.server_info.try_lock().unwrap().is_none());
    }

    #[test]
    fn test_client_creation_stdio() {
        let client = McpClient::new(McpTransport::Stdio {
            command: "python".into(),
            args: vec!["-m".into(), "mcp_server".into()],
        });
        assert!(client.server_info.try_lock().unwrap().is_none());
    }
}
