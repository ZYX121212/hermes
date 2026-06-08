// crates/tools/src/toolset.rs
// 工具集（Toolset）分类系统：将工具按功能域分组，支持按平台/profile 启用/禁用。

use std::collections::BTreeSet;
use std::sync::Arc;

use dashmap::DashMap;

use crate::{Tool, ToolOutput};

/// 工具类别标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ToolsetKind {
    Terminal,
    File,
    Browser,
    Web,
    CodeExec,
    Search,
    Vision,
    Finance,
    Mcp,
    Reply,
    Plugin,
}

impl ToolsetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::File => "file",
            Self::Browser => "browser",
            Self::Web => "web",
            Self::CodeExec => "code_exec",
            Self::Search => "search",
            Self::Vision => "vision",
            Self::Finance => "finance",
            Self::Mcp => "mcp",
            Self::Reply => "reply",
            Self::Plugin => "plugin",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Terminal => "Shell command execution tools",
            Self::File => "Filesystem read/write tools",
            Self::Browser => "Headless browser automation tools",
            Self::Web => "Web fetch and API tools",
            Self::CodeExec => "Sandboxed code execution tools",
            Self::Search => "Search engine integration tools",
            Self::Vision => "Image analysis and vision tools",
            Self::Finance => "Financial data provider tools",
            Self::Mcp => "Model Context Protocol tools",
            Self::Reply => "User-facing response tools",
            Self::Plugin => "User-installed plugin tools",
        }
    }
}

/// 线程安全的工具集注册中心：在 ToolRegistry 基础上增加工具集维度的管理。
pub struct ToolsetRegistry {
    /// 每个工具集当前是否启用
    enabled: DashMap<ToolsetKind, bool>,
    /// 底层工具注册表
    tools: DashMap<String, Arc<dyn Tool>>,
    /// 工具名 → 所属工具集
    tool_to_set: DashMap<String, ToolsetKind>,
}

impl Default for ToolsetRegistry {
    fn default() -> Self {
        let enabled = DashMap::new();
        // 默认启用所有工具集
        for kind in ALL_TOOLSETS {
            enabled.insert(*kind, true);
        }
        // 安全敏感的工具集默认禁用
        enabled.insert(ToolsetKind::Browser, false);
        enabled.insert(ToolsetKind::CodeExec, false);

        Self {
            enabled,
            tools: DashMap::new(),
            tool_to_set: DashMap::new(),
        }
    }
}

impl ToolsetRegistry {
    /// 注册工具到指定工具集。
    pub fn register(&self, toolset: ToolsetKind, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        tracing::debug!("registering tool [{:?}] {}", toolset, name);
        self.tools.insert(name.clone(), tool);
        self.tool_to_set.insert(name, toolset);
    }

    /// 启用/禁用某个工具集。
    pub fn set_enabled(&self, kind: ToolsetKind, on: bool) {
        self.enabled.insert(kind, on);
    }

    /// 查询工具集是否启用。
    pub fn is_enabled(&self, kind: ToolsetKind) -> bool {
        self.enabled.get(&kind).map(|v| *v).unwrap_or(false)
    }

    /// 调用工具（受工具集启用状态约束）。
    pub async fn call(&self, name: &str, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let toolset = self
            .tool_to_set
            .get(name)
            .map(|r| *r)
            .ok_or_else(|| anyhow::anyhow!("tool not found: {name}"))?;

        if !self.is_enabled(toolset) {
            return Err(anyhow::anyhow!(
                "toolset {:?} is disabled; tool '{name}' unavailable",
                toolset
            ));
        }

        let tool = self.tools.get(name).unwrap(); // 刚查过必定存在
        tool.call(args).await
    }

    /// 生成当前启用的所有工具描述（供 LLM prompt）。
    pub fn describe_all(&self) -> Vec<serde_json::Value> {
        self.tools
            .iter()
            .filter(|entry| {
                self.tool_to_set
                    .get(entry.name())
                    .map(|t| self.is_enabled(*t))
                    .unwrap_or(false)
            })
            .map(|entry| {
                let toolset = self
                    .tool_to_set
                    .get(entry.name())
                    .map(|t| t.as_str().to_string())
                    .unwrap_or_default();
                serde_json::json!({
                    "name": entry.name(),
                    "description": entry.description(),
                    "parameters": entry.schema(),
                    "toolset": toolset,
                })
            })
            .collect()
    }

    /// 按工具集分组描述工具。
    pub fn describe_by_toolset(&self) -> Vec<serde_json::Value> {
        let mut groups: Vec<(ToolsetKind, Vec<serde_json::Value>)> = Vec::new();
        for entry in self.tools.iter() {
            if let Some(ts) = self.tool_to_set.get(entry.name()) {
                if !self.is_enabled(*ts) {
                    continue;
                }
                let desc = serde_json::json!({
                    "name": entry.name(),
                    "description": entry.description(),
                    "parameters": entry.schema(),
                });
                if let Some(pos) = groups.iter().position(|(k, _)| *k == *ts) {
                    groups[pos].1.push(desc);
                } else {
                    groups.push((*ts, vec![desc]));
                }
            }
        }
        groups.sort_by_key(|(k, _)| *k);
        groups
            .into_iter()
            .map(|(kind, tools)| {
                serde_json::json!({
                    "toolset": kind.as_str(),
                    "description": kind.description(),
                    "enabled": self.is_enabled(kind),
                    "tools": tools,
                })
            })
            .collect()
    }

    /// 列出启用的工具集。
    pub fn enabled_toolsets(&self) -> Vec<ToolsetKind> {
        let sets: BTreeSet<_> = self.tool_to_set.iter().map(|e| *e.value()).collect();
        sets.into_iter().filter(|k| self.is_enabled(*k)).collect()
    }

    /// 工具总数（仅已启用的工具集）。
    pub fn len(&self) -> usize {
        self.tools
            .iter()
            .filter(|entry| {
                self.tool_to_set
                    .get(entry.name())
                    .map(|t| self.is_enabled(*t))
                    .unwrap_or(false)
            })
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 原始工具数（含禁用工具集）。
    pub fn total_len(&self) -> usize {
        self.tools.len()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
            && self
                .tool_to_set
                .get(name)
                .map(|t| self.is_enabled(*t))
                .unwrap_or(false)
    }
}

const ALL_TOOLSETS: &[ToolsetKind] = &[
    ToolsetKind::Terminal,
    ToolsetKind::File,
    ToolsetKind::Browser,
    ToolsetKind::Web,
    ToolsetKind::CodeExec,
    ToolsetKind::Search,
    ToolsetKind::Vision,
    ToolsetKind::Finance,
    ToolsetKind::Mcp,
    ToolsetKind::Reply,
    ToolsetKind::Plugin,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolOutput;
    use async_trait::async_trait;

    struct DummyTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "dummy"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn call(&self, _args: serde_json::Value) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::text("ok".into()))
        }
    }

    #[test]
    fn test_register_and_call() {
        let reg = ToolsetRegistry::default();
        reg.set_enabled(ToolsetKind::Reply, true);
        let tool = Arc::new(DummyTool { name: "dummy" });
        reg.register(ToolsetKind::Reply, tool);

        assert!(reg.contains("dummy"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_disabled_toolset() {
        let reg = ToolsetRegistry::default();
        reg.set_enabled(ToolsetKind::CodeExec, false);
        let tool = Arc::new(DummyTool { name: "code_exec" });
        reg.register(ToolsetKind::CodeExec, tool);

        assert!(!reg.contains("code_exec"));
        assert_eq!(reg.len(), 0);
    }

    #[tokio::test]
    async fn test_call_disabled_errors() {
        let reg = ToolsetRegistry::default();
        reg.set_enabled(ToolsetKind::Browser, false);
        let tool = Arc::new(DummyTool { name: "browser" });
        reg.register(ToolsetKind::Browser, tool);

        let result = reg.call("browser", serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_describe_respects_enabled() {
        let reg = ToolsetRegistry::default();
        reg.set_enabled(ToolsetKind::Reply, true);
        reg.set_enabled(ToolsetKind::Search, false);
        reg.register(ToolsetKind::Reply, Arc::new(DummyTool { name: "reply" }));
        reg.register(ToolsetKind::Search, Arc::new(DummyTool { name: "search" }));

        let desc = reg.describe_all();
        assert_eq!(desc.len(), 1);
        assert_eq!(desc[0]["name"], "reply");
    }

    #[test]
    fn test_enabled_toolsets() {
        let reg = ToolsetRegistry::default();
        reg.register(ToolsetKind::Reply, Arc::new(DummyTool { name: "r" }));
        reg.register(ToolsetKind::File, Arc::new(DummyTool { name: "f" }));

        let sets = reg.enabled_toolsets();
        // Reply and File are enabled by default; Browser/CodeExec are disabled
        assert!(sets.contains(&ToolsetKind::Reply));
        assert!(sets.contains(&ToolsetKind::File));
    }
}
