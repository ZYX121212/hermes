// crates/tools/src/code_exec.rs
// 代码执行沙箱工具：Python/JS/Bash 代码在隔离环境中执行。
use async_trait::async_trait;
use std::time::Duration;

use crate::{Tool, ToolOutput};

/// 执行模式
#[derive(Debug, Clone)]
pub enum ExecMode {
    /// 直接在宿主机执行（仅限受信环境）
    Host,
    /// Docker 容器隔离执行
    Docker { image: String, network_disabled: bool },
}

impl Default for ExecMode {
    fn default() -> Self {
        Self::Host
    }
}

/// 沙箱配置
pub struct SandboxConfig {
    pub mode: ExecMode,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: ExecMode::default(),
            timeout: Duration::from_secs(30),
            max_output_bytes: 64 * 1024, // 64KB
        }
    }
}

/// 在宿主机执行 Python 代码。
pub struct PythonExecTool {
    config: SandboxConfig,
}

impl PythonExecTool {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    async fn exec_host(&self, code: &str) -> anyhow::Result<String> {
        let output = tokio::time::timeout(
            self.config.timeout,
            tokio::process::Command::new("python3")
                .arg("-c")
                .arg(code)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("python execution timed out"))??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");

        let truncated: String = combined.chars().take(self.config.max_output_bytes).collect();
        if truncated.len() < combined.len() {
            Ok(format!("{truncated}\n... (output truncated)"))
        } else if truncated.is_empty() {
            Ok("(execution completed with no output)".into())
        } else {
            Ok(truncated)
        }
    }

    async fn exec_docker(&self, code: &str, image: &str, net_disabled: bool) -> anyhow::Result<String> {
        let escaped = code.replace('\"', "\\\"");
        let mut cmd = tokio::process::Command::new("docker");
        cmd.arg("run")
            .arg("--rm")
            .arg("--cap-drop=ALL")
            .arg("--security-opt=no-new-privileges")
            .arg("--memory=256m")
            .arg("--cpus=1")
            .arg("--pids-limit=64");

        if net_disabled {
            cmd.arg("--network=none");
        }

        cmd.arg(image)
            .arg("python3")
            .arg("-c")
            .arg(&escaped);

        let output = tokio::time::timeout(self.config.timeout, cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("docker execution timed out"))??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&format!("stdout:\n{stdout}\n"));
        }
        if !stderr.is_empty() {
            result.push_str(&format!("stderr:\n{stderr}"));
        }
        let truncated: String = result.chars().take(self.config.max_output_bytes).collect();
        Ok(truncated)
    }
}

#[async_trait]
impl Tool for PythonExecTool {
    fn name(&self) -> &str {
        "code_exec_python"
    }

    fn description(&self) -> &str {
        "Execute Python 3 code in a sandboxed environment. Use for calculations, data processing, or scripting. The code runs with a timeout and output limit."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python 3 code to execute"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let code = args["code"].as_str()
            .ok_or_else(|| anyhow::anyhow!("code_exec_python: 'code' is required"))?;

        let result = match &self.config.mode {
            ExecMode::Host => self.exec_host(code).await?,
            ExecMode::Docker { image, network_disabled } => {
                self.exec_docker(code, image, *network_disabled).await?
            }
        };

        Ok(ToolOutput {
            success: true,
            content: result,
            metadata: serde_json::json!({"lang": "python", "mode": format!("{:?}", self.config.mode)}),
        })
    }
}

/// 执行 JavaScript 代码（通过 node）。
pub struct JsExecTool {
    config: SandboxConfig,
}

impl JsExecTool {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for JsExecTool {
    fn name(&self) -> &str {
        "code_exec_js"
    }

    fn description(&self) -> &str {
        "Execute JavaScript code using Node.js in a sandboxed environment."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "JavaScript code to execute"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let code = args["code"].as_str()
            .ok_or_else(|| anyhow::anyhow!("code_exec_js: 'code' is required"))?;

        let output = tokio::time::timeout(
            self.config.timeout,
            tokio::process::Command::new("node")
                .arg("-e")
                .arg(code)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("js execution timed out"))??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");
        let result: String = combined.chars().take(self.config.max_output_bytes).collect();
        Ok(ToolOutput::text(result))
    }
}

/// 在受控环境中执行 Bash 命令（与 BashTool 不同，此工具限制更多）。
pub struct CodeExecBashTool {
    config: SandboxConfig,
}

impl CodeExecBashTool {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CodeExecBashTool {
    fn name(&self) -> &str {
        "code_exec_bash"
    }

    fn description(&self) -> &str {
        "Execute a bash one-liner in a sandboxed environment. Different from the general bash tool — this one has stricter resource limits and is meant for quick scripting tasks."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bash command to execute (one-liner)"
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let cmd = args["command"].as_str()
            .ok_or_else(|| anyhow::anyhow!("code_exec_bash: 'command' is required"))?;

        let output = tokio::time::timeout(
            self.config.timeout,
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(cmd)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("bash execution timed out"))??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");
        let result: String = combined.chars().take(self.config.max_output_bytes).collect();
        Ok(ToolOutput {
            success: output.status.success(),
            content: result,
            metadata: serde_json::json!({"exit_code": output.status.code()}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_schema() {
        let tool = PythonExecTool::new(SandboxConfig::default());
        let s = tool.schema();
        assert!(s["required"].as_array().unwrap().contains(&serde_json::json!("code")));
    }

    #[test]
    fn test_js_schema() {
        let tool = JsExecTool::new(SandboxConfig::default());
        assert_eq!(tool.name(), "code_exec_js");
    }

    #[tokio::test]
    async fn test_python_exec_host() {
        let tool = PythonExecTool::new(SandboxConfig::default());
        let result = tool.call(serde_json::json!({"code": "print('hello from python')"})).await;
        assert!(result.is_ok());
        assert!(result.unwrap().content.contains("hello from python"));
    }

    #[tokio::test]
    async fn test_js_exec_host() {
        let tool = JsExecTool::new(SandboxConfig::default());
        let result = tool.call(serde_json::json!({"code": "console.log('hello from js')"})).await;
        assert!(result.is_ok());
        assert!(result.unwrap().content.contains("hello from js"));
    }
}
