// crates/tools/src/plugin.rs
// Extensible plugin system: TOML-defined tools discovered from directories.
use async_trait::async_trait;
use serde::Deserialize;
use std::process::Stdio;

use crate::{Tool, ToolOutput};

/// TOML manifest that defines a plugin tool.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub description: String,
    /// "shell" or "script"
    #[serde(rename = "type")]
    pub plugin_type: String,
    /// JSON schema for tool arguments
    #[serde(default)]
    pub schema: serde_json::Value,
    /// Shell command template with `$ARG.field` substitution
    pub command: Option<String>,
    /// Interpreter (e.g. "python3", "node") for script plugins
    pub interpreter: Option<String>,
    /// Path to script file (relative to manifest)
    pub script: Option<String>,
}

/// A plugin tool backed by a shell command template.
pub struct ShellPlugin {
    manifest: PluginManifest,
}

impl ShellPlugin {
    pub fn new(manifest: PluginManifest) -> Self {
        Self { manifest }
    }

    fn resolve_args(&self, args: &serde_json::Value) -> String {
        let mut cmd = self.manifest.command.clone().unwrap_or_default();
        if let Some(obj) = args.as_object() {
            for (key, val) in obj {
                let placeholder = format!("$ARG.{}", key);
                let val_str = {
                    let s = match val {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    let escaped = s.replace('\'', "'\\''");
                    format!("'{}'", escaped)
                };
                cmd = cmd.replace(&placeholder, &val_str);
            }
        }
        cmd
    }
}

#[async_trait]
impl Tool for ShellPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn schema(&self) -> serde_json::Value {
        self.manifest.schema.clone()
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let resolved = self.resolve_args(&args);
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&resolved)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let content = if stdout.is_empty() { stderr } else { stdout };

        Ok(ToolOutput {
            success: output.status.success(),
            content,
            metadata: serde_json::json!({
                "exit_code": output.status.code(),
                "plugin": self.manifest.name,
            }),
        })
    }
}

/// A plugin tool backed by a script file (e.g. Python, Node).
/// JSON args are piped via stdin; stdout is captured as the result.
pub struct ScriptPlugin {
    manifest: PluginManifest,
    script_dir: std::path::PathBuf,
}

impl ScriptPlugin {
    pub fn new(manifest: PluginManifest, script_dir: std::path::PathBuf) -> Self {
        Self {
            manifest,
            script_dir,
        }
    }
}

#[async_trait]
impl Tool for ScriptPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn schema(&self) -> serde_json::Value {
        self.manifest.schema.clone()
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let interpreter = self.manifest.interpreter.as_deref().unwrap_or("sh");
        if !matches!(interpreter, "sh" | "bash" | "python3" | "python" | "node" | "ruby" | "perl") {
            return Ok(ToolOutput::error(format!(
                "不支持的脚本解释器: {interpreter}。允许: sh, bash, python3, python, node, ruby, perl"
            )));
        }
        let script_path = self
            .script_dir
            .join(self.manifest.script.as_deref().unwrap_or("run.sh"));

        let args_json = serde_json::to_string(&args)?;

        let mut child = tokio::process::Command::new(interpreter)
            .arg(&script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Write JSON args to stdin
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(args_json.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let content = if stdout.is_empty() { stderr } else { stdout };

        Ok(ToolOutput {
            success: output.status.success(),
            content,
            metadata: serde_json::json!({
                "exit_code": output.status.code(),
                "plugin": self.manifest.name,
            }),
        })
    }
}

/// Discover and load plugin manifests from a directory.
/// Each subdirectory containing a `plugin.toml` is treated as a plugin.
pub fn discover_plugins(dir: &str) -> anyhow::Result<Vec<(PluginManifest, std::path::PathBuf)>> {
    let plugin_dir = std::path::Path::new(dir);
    if !plugin_dir.exists() {
        tracing::info!(dir = %dir, "Plugin directory not found, skipping");
        return Ok(vec![]);
    }

    let mut plugins = Vec::new();
    for entry in std::fs::read_dir(plugin_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("plugin.toml");
        if !manifest_path.exists() {
            continue;
        }
        match std::fs::read_to_string(&manifest_path) {
            Ok(toml_str) => match toml::from_str::<PluginManifest>(&toml_str) {
                Ok(manifest) => {
                    tracing::info!(
                        name = %manifest.name,
                        plugin_type = %manifest.plugin_type,
                        dir = %path.display(),
                        "发现插件"
                    );
                    plugins.push((manifest, path));
                }
                Err(e) => {
                    tracing::warn!(
                        path = %manifest_path.display(),
                        error = %e,
                        "插件清单解析失败"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "无法读取插件清单"
                );
            }
        }
    }
    Ok(plugins)
}
