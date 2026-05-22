// crates/tools/src/registry.rs
use std::sync::Arc;

use dashmap::DashMap;

use crate::{Tool, ToolOutput};

/// Thread-safe registry of available tools.
/// Supports dynamic registration and lookup by name.
pub struct ToolRegistry {
    tools: DashMap<String, Arc<dyn Tool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: DashMap::new(),
        }
    }
}

impl ToolRegistry {
    /// Register a tool under its name.
    pub fn register(&self, tool: Arc<dyn Tool>) {
        tracing::debug!("registering tool: {}", tool.name());
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Call a tool by name with the given arguments.
    pub async fn call(&self, name: &str, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("tool not found: {name}"))?;
        tool.call(args).await
    }

    /// Generate a JSON description of all registered tools (for LLM prompts).
    pub fn describe_all(&self) -> Vec<serde_json::Value> {
        self.tools
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name(),
                    "description": entry.description(),
                    "parameters": entry.schema()
                })
            })
            .collect()
    }

    /// Check if a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
