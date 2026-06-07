// crates/tools/src/builtin/file.rs
// File system tools: read_file and write_file.
use crate::{Tool, ToolOutput};
use async_trait::async_trait;

/// Reads the contents of a file at the given path.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at a given path. Returns the file content as text."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let path = args["path"].as_str().unwrap_or("");
        if path.is_empty() {
            return Ok(ToolOutput::error("path is required".into()));
        }

        tracing::info!("ReadFileTool reading: {path}");

        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                let truncated = if content.len() > 10000 {
                    let end = content
                        .char_indices()
                        .take_while(|&(i, _)| i < 10000)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(0);
                    format!(
                        "{}...\n[truncated, {} bytes total]",
                        &content[..end],
                        content.len()
                    )
                } else {
                    content
                };
                Ok(ToolOutput {
                    success: true,
                    content: truncated,
                    metadata: serde_json::json!({"path": path}),
                })
            }
            Err(e) => Ok(ToolOutput::error(format!("read failed: {e}"))),
        }
    }
}

/// Writes content to a file at the given path.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let path = args["path"].as_str().unwrap_or("");
        let content = args["content"].as_str().unwrap_or("");

        if path.is_empty() {
            return Ok(ToolOutput::error("path is required".into()));
        }

        tracing::info!("WriteFileTool writing to: {path}");

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        match tokio::fs::write(path, content).await {
            Ok(_) => Ok(ToolOutput {
                success: true,
                content: format!("Successfully wrote {} bytes to {path}", content.len()),
                metadata: serde_json::json!({"path": path, "bytes_written": content.len()}),
            }),
            Err(e) => Ok(ToolOutput::error(format!("write failed: {e}"))),
        }
    }
}
