// crates/mcp/src/server.rs
// MCP stdio server: reads JSON-RPC requests from stdin, writes responses to stdout.
use crate::protocol::*;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Handler trait for MCP requests. Implement this to provide tool capabilities.
#[async_trait::async_trait]
pub trait McpHandler: Send + Sync {
    /// Return the list of available tools.
    async fn list_tools(&self) -> anyhow::Result<Vec<ToolDef>>;
    /// Call a tool by name with arguments.
    async fn call_tool(&self, name: &str, args: Option<Value>) -> anyhow::Result<ToolCallResult>;
}

/// Run an MCP server over stdio. Reads JSON-RPC requests from stdin
/// and writes responses to stdout. Stderr is used for logging.
pub async fn run_stdio_server(handler: Box<dyn McpHandler>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout_handle = stdout;

    // Send initialize response
    let init_resp = JsonRpcResponse::success(
        Value::Number(0.into()),
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "hermes-mcp",
                "version": "0.1.0"
            }
        }),
    );
    let resp_line = serde_json::to_string(&init_resp)?;
    stdout_handle.write_all(resp_line.as_bytes()).await?;
    stdout_handle.write_all(b"\n").await?;
    stdout_handle.flush().await?;

    // Wait for initialized notification
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err = JsonRpcResponse::error(None, PARSE_ERROR, &format!("Parse error: {e}"));
                write_response(&err, &mut stdout_handle).await?;
                continue;
            }
        };

        if req.method == METHOD_NOTIFICATION_INITIALIZED {
            tracing::info!("MCP client initialized");
            break;
        }
    }

    // Main request loop
    tracing::info!("MCP server ready, waiting for requests");
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err = JsonRpcResponse::error(None, PARSE_ERROR, &format!("Parse error: {e}"));
                write_response(&err, &mut stdout_handle).await?;
                continue;
            }
        };

        // Skip notifications
        let _id = match req.id.clone() {
            Some(id) => id,
            None => continue,
        };

        let response = handle_request(&req, handler.as_ref()).await;
        write_response(&response, &mut stdout_handle).await?;
    }

    Ok(())
}

async fn handle_request(req: &JsonRpcRequest, handler: &dyn McpHandler) -> JsonRpcResponse {
    let id = req.id.clone();

    match req.method.as_str() {
        METHOD_TOOLS_LIST => match handler.list_tools().await {
            Ok(tools) => {
                let tools_json: Vec<Value> = tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema,
                        })
                    })
                    .collect();
                JsonRpcResponse::success(id.unwrap(), serde_json::json!({ "tools": tools_json }))
            }
            Err(e) => JsonRpcResponse::error(id, INTERNAL_ERROR, &e.to_string()),
        },
        METHOD_TOOLS_CALL => {
            let params: ToolCallParams = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
                Some(p) => p,
                None => return JsonRpcResponse::error(id, INVALID_PARAMS, "invalid tool call params"),
            };
            match handler.call_tool(&params.name, params.arguments).await {
                Ok(result) => JsonRpcResponse::success(id.unwrap(), serde_json::to_value(result).unwrap_or_default()),
                Err(e) => JsonRpcResponse::error(id, INTERNAL_ERROR, &e.to_string()),
            }
        }
        _ => JsonRpcResponse::error(id, METHOD_NOT_FOUND, &format!("unknown method: {}", req.method)),
    }
}

async fn write_response(
    resp: &JsonRpcResponse,
    stdout: &mut tokio::io::Stdout,
) -> anyhow::Result<()> {
    let line = serde_json::to_string(resp)?;
    stdout.write_all(line.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}
