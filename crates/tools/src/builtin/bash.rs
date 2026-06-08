// crates/tools/src/builtin/bash.rs
use std::sync::Arc;

use crate::{Tool, ToolOutput};
use async_trait::async_trait;

use super::guard::{ConfirmationPolicy, DangerGuard};

/// 执行 shell 命令（通过 bash -c），内建安全守卫。
pub struct BashTool {
    guard: Arc<DangerGuard>,
}

impl BashTool {
    /// 创建一个带有安全守卫的 BashTool。
    pub fn new(guard: Arc<DangerGuard>) -> Self {
        Self { guard }
    }

    /// 创建一个无守卫的 BashTool（向后兼容，不推荐）。
    pub fn unguarded() -> Self {
        Self {
            guard: Arc::new(DangerGuard::new(ConfirmationPolicy::Auto, vec![])),
        }
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            guard: Arc::new(DangerGuard::new(ConfirmationPolicy::Deny, vec![])),
        }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "执行 bash 命令并返回 stdout/stderr。危险命令需要用户确认。"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "要执行的 shell 命令。避免危险操作如 rm -rf, sudo, chmod 777 等，这些需要用户确认。"
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

        // ── 安全守卫检查 ──
        if self.guard.is_dangerous(cmd) {
            if self.guard.policy() == ConfirmationPolicy::Deny {
                let msg = format!(
                    "危险命令已被安全策略自动拒绝: {}",
                    DangerGuard::summarize(cmd)
                );
                tracing::warn!("{msg}");
                return Ok(ToolOutput::error(msg));
            }
            // Ask 模式：当前无交互式确认通道，拒绝执行以确保安全
            // 上层（TUI/CLI）应在调用前自行确认，或使用 Deny/Skip 策略
            if self.guard.policy() == ConfirmationPolicy::Ask {
                let msg = format!(
                    "危险命令需要用户确认后才能执行: {}。请使用更安全的替代命令，或调整安全策略。",
                    DangerGuard::summarize(cmd)
                );
                tracing::warn!("{msg}");
                return Ok(ToolOutput::error(msg));
            }
        }

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
            Err(e) => Ok(ToolOutput::error(format!("bash 执行失败: {e}"))),
        }
    }
}
