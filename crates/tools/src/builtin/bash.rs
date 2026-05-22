// crates/tools/src/builtin/bash.rs
use async_trait::async_trait;
use crate::{Tool, ToolOutput};

/// Executes shell commands via bash.
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a bash command and return stdout/stderr."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let cmd = args["command"]
            .as_str()
            .unwrap_or("echo 'no command provided'");

        tracing::info!("BashTool executing: {cmd}");

        match tokio::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n{stderr}")
                };
                Ok(ToolOutput {
                    success: output.status.success(),
                    content: combined,
                    metadata: serde_json::json!({
                        "exit_code": output.status.code(),
                        "stdout_len": output.stdout.len(),
                        "stderr_len": output.stderr.len()
                    }),
                })
            }
            Err(e) => Ok(ToolOutput::error(format!("bash execution failed: {e}"))),
        }
    }
}
