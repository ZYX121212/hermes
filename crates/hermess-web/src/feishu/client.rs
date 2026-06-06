// crates/hermess-web/src/feishu/client.rs
// 飞书 REST API 客户端 — tenant_access_token 管理 + 消息发送
use anyhow::{bail, Context};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

pub struct FeishuClient {
    app_id: String,
    app_secret: String,
    http: reqwest::Client,
    token_cache: RwLock<CachedToken>,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

const OPEN_API_BASE: &str = "https://open.feishu.cn";

impl FeishuClient {
    pub fn new(app_id: String, app_secret: String) -> Arc<Self> {
        Arc::new(Self {
            app_id,
            app_secret,
            http: reqwest::Client::new(),
            token_cache: RwLock::new(CachedToken {
                token: String::new(),
                expires_at: Instant::now(),
            }),
        })
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// 获取 tenant_access_token，自动缓存+提前5分钟刷新
    pub async fn get_tenant_access_token(&self) -> anyhow::Result<String> {
        {
            let cache = self.token_cache.read().await;
            if !cache.token.is_empty()
                && cache.expires_at > Instant::now() + std::time::Duration::from_secs(300)
            {
                return Ok(cache.token.clone());
            }
        }

        let mut cache = self.token_cache.write().await;
        // 双重检查
        if !cache.token.is_empty()
            && cache.expires_at > Instant::now() + std::time::Duration::from_secs(300)
        {
            return Ok(cache.token.clone());
        }

        let resp = self
            .http
            .post(format!("{}/open-apis/auth/v3/tenant_access_token/internal", OPEN_API_BASE))
            .json(&serde_json::json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await
            .context("failed to fetch tenant_access_token")?;

        #[derive(Deserialize)]
        struct TokenResp {
            code: i32,
            msg: Option<String>,
            tenant_access_token: Option<String>,
            expire: Option<u64>,
        }

        let tr: TokenResp = resp.json().await.context("failed to parse token response")?;
        if tr.code != 0 {
            bail!(
                "get tenant_access_token failed: code={} msg={:?}",
                tr.code,
                tr.msg
            );
        }

        let token = tr.tenant_access_token.unwrap_or_default();
        let expire = tr.expire.unwrap_or(7200);

        *cache = CachedToken {
            token: token.clone(),
            expires_at: Instant::now() + std::time::Duration::from_secs(expire),
        };

        tracing::info!(expire_secs = expire, "tenant_access_token refreshed");
        Ok(token)
    }

    /// 获取 WebSocket 连接 URL
    pub async fn get_ws_url(&self) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/ws/v1/url", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get ws url")?;

        #[derive(Deserialize)]
        struct WsResp {
            code: i32,
            msg: Option<String>,
            data: Option<WsData>,
        }
        #[derive(Deserialize)]
        struct WsData {
            url: String,
        }

        let wr: WsResp = resp.json().await.context("failed to parse ws url response")?;
        if wr.code != 0 {
            bail!("get ws url failed: code={} msg={:?}", wr.code, wr.msg);
        }

        let url = wr.data.ok_or_else(|| anyhow::anyhow!("ws url data missing"))?.url;
        tracing::info!(%url, "got ws url");
        Ok(url)
    }

    /// 回复消息（被动回复，需要在收到消息后 1 小时内）
    pub async fn reply_text(&self, message_id: &str, content: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let url = format!(
            "{}/open-apis/im/v1/messages/{}/reply",
            OPEN_API_BASE, message_id
        );

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "content": serde_json::json!({"text": content}).to_string(),
                "msg_type": "text",
            }))
            .send()
            .await
            .context("failed to reply message")?;

        #[derive(Deserialize)]
        struct ReplyResp {
            code: i32,
            msg: Option<String>,
        }

        let rr: ReplyResp = resp.json().await.context("failed to parse reply response")?;
        if rr.code != 0 {
            bail!("reply failed: code={} msg={:?}", rr.code, rr.msg);
        }

        tracing::info!(message_id = %message_id, len = content.len(), "reply sent");
        Ok(())
    }

    // ── Wiki 知识库 API ──────────────────────────────────────

    pub async fn list_wiki_spaces(
        &self,
        page_token: Option<&str>,
        page_size: Option<u64>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/wiki/v2/spaces", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .query(&[
                ("page_token", page_token.unwrap_or("")),
                ("page_size", &page_size.unwrap_or(20).to_string()),
            ])
            .send()
            .await
            .context("failed to list wiki spaces")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn get_wiki_space_detail(&self, space_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/wiki/v2/spaces/{space_id}", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get wiki space detail")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn get_wiki_node_tree(
        &self,
        space_id: &str,
        parent_node_token: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let mut query = vec![];
        if let Some(pt) = parent_node_token {
            query.push(("parent_node_token", pt.to_string()));
        }
        let resp = self
            .http
            .get(format!("{}/open-apis/wiki/v2/spaces/{space_id}/nodes", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .query(&query.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>())
            .send()
            .await
            .context("failed to get wiki node tree")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn get_wiki_node_content(&self, node_token: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/wiki/v2/spaces/-/nodes/{node_token}", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get wiki node")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let content = body["data"]["node"]["content"].as_str().unwrap_or("").to_string();
        Ok(content)
    }

    pub async fn create_wiki_node(
        &self,
        space_id: &str,
        parent_node_token: &str,
        title: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .post(format!("{}/open-apis/wiki/v2/spaces/{space_id}/nodes", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "parent_node_token": parent_node_token,
                "node_type": "docx",
                "title": title,
                "content": content,
            }))
            .send()
            .await
            .context("failed to create wiki node")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn update_wiki_node(
        &self,
        node_token: &str,
        title: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .put(format!("{}/open-apis/wiki/v2/spaces/-/nodes/{node_token}", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({ "title": title, "content": content }))
            .send()
            .await
            .context("failed to update wiki node")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn search_wiki(
        &self,
        query: &str,
        space_ids: Option<&[String]>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let mut body_json = serde_json::json!({ "query": query });
        if let Some(ids) = space_ids {
            body_json["space_ids"] = serde_json::json!(ids);
        }
        let resp = self
            .http
            .post(format!("{}/open-apis/wiki/v2/search", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .json(&body_json)
            .send()
            .await
            .context("failed to search wiki")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    // ── Docs 文档 API ──────────────────────────────────────

    pub async fn get_document(&self, doc_token: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/docx/v1/documents/{doc_token}", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get document")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let content = body["data"]["document"]["content"].as_str().unwrap_or("").to_string();
        Ok(content)
    }

    pub async fn create_document(&self, title: &str, folder_token: Option<&str>) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let mut json = serde_json::json!({ "title": title });
        if let Some(ft) = folder_token {
            json["folder_token"] = serde_json::json!(ft);
        }
        let resp = self
            .http
            .post(format!("{}/open-apis/docx/v1/documents", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .json(&json)
            .send()
            .await
            .context("failed to create document")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let doc_token = body["data"]["document"]["document_id"].as_str().unwrap_or("").to_string();
        Ok(doc_token)
    }

    pub async fn get_document_blocks(&self, doc_token: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/docx/v1/documents/{doc_token}/blocks", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get document blocks")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn append_document_blocks(
        &self,
        doc_token: &str,
        block_id: &str,
        blocks: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .post(format!(
                "{}/open-apis/docx/v1/documents/{doc_token}/blocks/{block_id}/children",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({ "children": blocks }))
            .send()
            .await
            .context("failed to append document blocks")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn update_document_block(
        &self,
        doc_token: &str,
        block_id: &str,
        update: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .patch(format!(
                "{}/open-apis/docx/v1/documents/{doc_token}/blocks/{block_id}",
                OPEN_API_BASE
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(update)
            .send()
            .await
            .context("failed to update document block")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    // ── Drive 云盘 API ──────────────────────────────────────

    pub async fn list_drive_files(
        &self,
        folder_token: Option<&str>,
        page_size: Option<u64>,
        page_token: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let mut query: Vec<(&str, String)> = vec![];
        if let Some(ft) = folder_token { query.push(("folder_token", ft.to_string())); }
        if let Some(ps) = page_size { query.push(("page_size", ps.to_string())); }
        if let Some(pt) = page_token { query.push(("page_token", pt.to_string())); }
        let resp = self
            .http
            .get(format!("{}/open-apis/drive/v1/files", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .query(&query.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>())
            .send()
            .await
            .context("failed to list drive files")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn upload_drive_file(
        &self,
        folder_token: &str,
        file_name: &str,
        data: Vec<u8>,
        mime_type: &str,
    ) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(file_name.to_string())
            .mime_str(mime_type)
            .context("invalid mime type")?;
        let form = reqwest::multipart::Form::new()
            .text("folder_token", folder_token.to_string())
            .text("file_name", file_name.to_string())
            .part("file", part);
        let resp = self
            .http
            .post(format!("{}/open-apis/drive/v1/files/upload_all", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .context("failed to upload file")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        let file_token = body["data"]["file_token"].as_str().unwrap_or("").to_string();
        Ok(file_token)
    }

    pub async fn download_drive_file(&self, file_token: &str) -> anyhow::Result<Vec<u8>> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/drive/v1/files/{file_token}/download", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to download file")?;
        let bytes = resp.bytes().await.context("failed to read file bytes")?;
        Ok(bytes.to_vec())
    }

    pub async fn get_drive_file_info(&self, file_token: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .get(format!("{}/open-apis/drive/v1/files/{file_token}", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to get file info")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(body)
    }

    pub async fn delete_drive_file(&self, file_token: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .http
            .delete(format!("{}/open-apis/drive/v1/files/{file_token}", OPEN_API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("failed to delete file")?;
        let body: serde_json::Value = resp.json().await?;
        check_feishu_code(&body)?;
        Ok(())
    }
}

/// 检查飞书 API 响应的 code 字段，非 0 返回错误
fn check_feishu_code(body: &serde_json::Value) -> anyhow::Result<()> {
    let code = body["code"].as_i64().unwrap_or(-1);
    if code != 0 {
        let msg = body["msg"].as_str().unwrap_or("unknown error");
        anyhow::bail!("feishu api error: code={code} msg={msg}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = FeishuClient::new("app_001".into(), "secret_001".into());
        assert_eq!(client.app_id(), "app_001");
    }
}
