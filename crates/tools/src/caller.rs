// crates/tools/src/caller.rs
use std::sync::Arc;
use std::time::Instant;

use crate::{ToolOutput, registry::ToolRegistry};

/// Unified tool-calling interface with timing instrumentation.
pub struct ToolCaller {
    registry: Arc<ToolRegistry>,
}

impl ToolCaller {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    /// Call a tool by name, measuring execution time.
    pub async fn call(&self, name: &str, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let start = Instant::now();
        let result = self.registry.call(name, args).await;
        let elapsed = start.elapsed();
        tracing::debug!("tool {name} completed in {elapsed:?}");
        result
    }
}
