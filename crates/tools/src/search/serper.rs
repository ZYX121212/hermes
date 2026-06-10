// crates/tools/src/search/serper.rs
// Serper.dev Google Search API 客户端 (https://serper.dev)

pub struct SerperClient {
    api_key: String,
    http: reqwest::Client,
}

impl SerperClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// 通用 Google 搜索
    pub async fn search(&self, query: &str, count: usize) -> anyhow::Result<String> {
        let resp = self
            .http
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "q": query,
                "num": count.min(20),
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        self.format_organic(&json)
    }

    /// 新闻搜索
    pub async fn search_news(&self, query: &str, count: usize) -> anyhow::Result<String> {
        let resp = self
            .http
            .post("https://google.serper.dev/news")
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "q": query,
                "num": count.min(20),
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        if let Some(news) = json["news"].as_array() {
            let mut out = String::from("News results:\n\n");
            for (i, item) in news.iter().enumerate() {
                let title = item["title"].as_str().unwrap_or("(untitled)");
                let link = item["link"].as_str().unwrap_or("");
                let snippet = item["snippet"].as_str().unwrap_or("");
                let date = item["date"].as_str().unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   Date: {date}\n   URL: {link}\n   {snippet}\n\n",
                    i + 1
                ));
            }
            if out == "News results:\n\n" {
                out = "(no news results)".into();
            }
            return Ok(out);
        }
        self.format_organic(&json)
    }

    /// 图片搜索
    pub async fn search_images(&self, query: &str, count: usize) -> anyhow::Result<String> {
        let resp = self
            .http
            .post("https://google.serper.dev/images")
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "q": query,
                "num": count.min(10),
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let mut out = String::from("Image search results:\n\n");
        if let Some(images) = json["images"].as_array() {
            for (i, img) in images.iter().enumerate() {
                let title = img["title"].as_str().unwrap_or("(untitled)");
                let url = img["imageUrl"].as_str().unwrap_or("");
                let source = img["source"].as_str().unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   Image: {url}\n   Source: {source}\n\n",
                    i + 1
                ));
            }
        }
        if out == "Image search results:\n\n" {
            out = "(no image results)".into();
        }
        Ok(out)
    }

    fn format_organic(&self, json: &serde_json::Value) -> anyhow::Result<String> {
        let mut out = String::new();
        if let Some(organic) = json["organic"].as_array() {
            for (i, r) in organic.iter().enumerate() {
                let title = r["title"].as_str().unwrap_or("(untitled)");
                let link = r["link"].as_str().unwrap_or("");
                let snippet = r["snippet"].as_str().unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   URL: {link}\n   {snippet}\n\n",
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
