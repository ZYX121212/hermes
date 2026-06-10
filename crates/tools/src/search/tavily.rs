// crates/tools/src/search/tavily.rs
// Tavily Search API 客户端 (https://tavily.com)

pub struct TavilyClient {
    api_key: String,
    http: reqwest::Client,
}

impl TavilyClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// 通用搜索
    pub async fn search(&self, query: &str, count: usize) -> anyhow::Result<String> {
        let resp = self
            .http
            .post("https://api.tavily.com/search")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "api_key": self.api_key,
                "query": query,
                "max_results": count.min(20),
                "include_answer": true,
                "include_raw_content": false,
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        self.format_results(&json)
    }

    /// 新闻专用搜索
    pub async fn search_news(&self, query: &str, count: usize) -> anyhow::Result<String> {
        let resp = self
            .http
            .post("https://api.tavily.com/search")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "api_key": self.api_key,
                "query": query,
                "max_results": count.min(20),
                "topic": "news",
                "days": 30,
                "include_answer": true,
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        self.format_results(&json)
    }

    fn format_results(&self, json: &serde_json::Value) -> anyhow::Result<String> {
        let mut out = String::new();

        if let Some(answer) = json["answer"].as_str() {
            out.push_str(&format!("Answer: {answer}\n\n"));
        }

        if let Some(results) = json["results"].as_array() {
            for (i, r) in results.iter().enumerate() {
                let title = r["title"].as_str().unwrap_or("(untitled)");
                let url = r["url"].as_str().unwrap_or("");
                let content = r["content"].as_str().unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   URL: {url}\n   {content}\n\n",
                    i + 1
                ));
            }
        }

        if out.is_empty() {
            out = "(no results)".into();
        }
        Ok(out)
    }
}
