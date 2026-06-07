// crates/tools/src/builtin/web_search.rs
// Web search tool using Brave Search API.
use async_trait::async_trait;
use serde::Deserialize;

use crate::{Tool, ToolOutput};

/// Configuration for the web search tool.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    pub api_key: Option<String>,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
}

fn default_endpoint() -> String {
    "https://api.search.brave.com/res/v1/web/search".to_string()
}

/// Searches the web via the Brave Search API.
pub struct WebSearchTool {
    client: reqwest::Client,
    config: SearchConfig,
}

impl WebSearchTool {
    /// Create a new WebSearchTool from configuration.
    pub fn new(cfg: &SearchConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config: cfg.clone(),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information and return relevant results."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "num_results": {
                    "type": "integer",
                    "description": "Number of results (default: 5)",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let query = args["query"].as_str().unwrap_or("");
        if query.is_empty() {
            return Ok(ToolOutput::error("search query is empty".into()));
        }

        tracing::info!("WebSearchTool query: {query}");

        // If no API key is configured, return a helpful message.
        let Some(ref api_key) = self.config.api_key else {
            return Ok(ToolOutput {
                success: true,
                content: format!(
                    "Web search not configured (no API key). Query: \"{query}\". \
                     Set search.api_key in config to enable web search."
                ),
                metadata: serde_json::json!({"configured": false}),
            });
        };

        // Perform search via Brave Search API
        let num_results = args["num_results"].as_u64().unwrap_or(5).min(20);
        let count_str = num_results.to_string();
        let resp = self
            .client
            .get(&self.config.endpoint)
            .query(&[("q", query), ("count", count_str.as_str())])
            .header("Accept", "application/json")
            .header("X-Subscription-Token", api_key.as_str())
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body = match r.text().await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(error = %e, "WebSearch response body read failed");
                        return Ok(ToolOutput::error(format!(
                            "Search response read failed: {e}"
                        )));
                    }
                };
                Ok(ToolOutput {
                    success: true,
                    content: body,
                    metadata: serde_json::json!({"configured": true}),
                })
            }
            Ok(r) => Ok(ToolOutput::error(format!(
                "Search API returned status {}",
                r.status()
            ))),
            Err(e) => Ok(ToolOutput::error(format!("Search request failed: {e}"))),
        }
    }
}
