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
