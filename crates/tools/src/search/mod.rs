// crates/tools/src/search/mod.rs
// 增强搜索工具集：网页抓取、新闻搜索、图片搜索，集成 Tavily/Serper API。
mod tavily;
mod serper;

pub use tavily::TavilyClient;
pub use serper::SerperClient;

use async_trait::async_trait;
use crate::{Tool, ToolOutput};

/// 网页抓取配置
pub struct FetchConfig {
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
    pub user_agent: String,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_body_bytes: 2 * 1024 * 1024, // 2MB
            user_agent: "Hermes/1.0 WebFetcher".into(),
        }
    }
}

/// 网页抓取工具：获取 URL 内容并提取为文本。
pub struct WebFetchTool {
    http: reqwest::Client,
    config: FetchConfig,
}

impl WebFetchTool {
    pub fn new(config: FetchConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(config.timeout_secs))
                .build()
                .unwrap_or_default(),
            config,
        }
    }

    async fn fetch_and_extract(&self, url: &str) -> anyhow::Result<String> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", &self.config.user_agent)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("fetch failed: HTTP {status}");
        }

        let body = resp.text().await?;
        let text = html2text::from_read(body.as_bytes(), 80);
        // 如果 html2text 返回空，回退到原始文本
        let text = if text.trim().is_empty() {
            body.chars().take(self.config.max_body_bytes).collect()
        } else {
            text
        };

        // 截断到限制
        if text.len() > self.config.max_body_bytes {
            Ok(text.chars().take(self.config.max_body_bytes).collect())
        } else {
            Ok(text)
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page URL and extract its main text content. Returns the page text (HTML converted to markdown-like format). Use this to read articles, documentation, or any web content."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch and extract content from"
                }
            },
            "required": ["url"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("web_fetch: 'url' is required"))?;

        let content = self.fetch_and_extract(url).await?;
        Ok(ToolOutput {
            success: true,
            content: format!("[Fetched from {url}]\n\n{content}"),
            metadata: serde_json::json!({"url": url, "length": content.len()}),
        })
    }
}

/// 新闻搜索工具：通过 Tavily 或 Serper 搜索新闻。
pub struct NewsSearchTool {
    tavily: Option<TavilyClient>,
    serper: Option<SerperClient>,
}

impl NewsSearchTool {
    pub fn new(tavily: Option<TavilyClient>, serper: Option<SerperClient>) -> Self {
        Self { tavily, serper }
    }

    async fn search(&self, query: &str, count: usize) -> anyhow::Result<String> {
        if let Some(ref t) = self.tavily {
            return t.search_news(query, count).await;
        }
        if let Some(ref s) = self.serper {
            return s.search_news(query, count).await;
        }
        anyhow::bail!("news_search: no search backend configured (set TAVILY_API_KEY or SERPER_API_KEY)")
    }
}

#[async_trait]
impl Tool for NewsSearchTool {
    fn name(&self) -> &str {
        "news_search"
    }

    fn description(&self) -> &str {
        "Search for recent news articles on a given topic. Returns article titles, snippets, and URLs. Best for finding current events and recent developments."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query for news articles"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (default 5, max 20)",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("news_search: 'query' is required"))?;
        let count = args["count"].as_u64().unwrap_or(5).min(20) as usize;
        let result = self.search(query, count).await?;
        Ok(ToolOutput::text(result))
    }
}

/// 图片搜索工具。
pub struct ImageSearchTool {
    serper: Option<SerperClient>,
}

impl ImageSearchTool {
    pub fn new(serper: Option<SerperClient>) -> Self {
        Self { serper }
    }
}

#[async_trait]
impl Tool for ImageSearchTool {
    fn name(&self) -> &str {
        "image_search"
    }

    fn description(&self) -> &str {
        "Search for images on the web. Returns image URLs and descriptions. Useful for finding visual references, diagrams, or photographs."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query for images"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (default 5, max 10)",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("image_search: 'query' is required"))?;
        let count = args["count"].as_u64().unwrap_or(5).min(10) as usize;

        if let Some(ref s) = self.serper {
            let result = s.search_images(query, count).await?;
            return Ok(ToolOutput::text(result));
        }
        anyhow::bail!("image_search: no search backend configured (set SERPER_API_KEY)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_fetch_schema() {
        let tool = WebFetchTool::new(FetchConfig::default());
        let schema = tool.schema();
        assert!(schema["required"].as_array().unwrap().contains(&serde_json::json!("url")));
    }

    #[test]
    fn test_news_search_schema() {
        let tool = NewsSearchTool::new(None, None);
        assert_eq!(tool.name(), "news_search");
    }

    #[test]
    fn test_image_search_schema() {
        let tool = ImageSearchTool::new(None);
        assert_eq!(tool.name(), "image_search");
    }
}
