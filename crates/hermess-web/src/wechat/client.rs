// 企业微信 API 客户端
// 参考: https://developer.work.weixin.qq.com/document/path/90236

use anyhow::{bail, Context};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// 企业微信 API 客户端，自动管理 access_token 缓存
pub struct WeChatClient {
    corp_id: String,
    agent_id: String,
    secret: String,
    http: reqwest::Client,
    token_cache: RwLock<CachedToken>,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

impl WeChatClient {
    pub fn new(corp_id: String, agent_id: String, secret: String) -> Arc<Self> {
        Arc::new(Self {
            corp_id,
            agent_id,
            secret,
            http: reqwest::Client::new(),
            token_cache: RwLock::new(CachedToken {
                token: String::new(),
                expires_at: Instant::now(),
            }),
        })
    }

    pub fn corp_id(&self) -> &str {
        &self.corp_id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// 获取 access_token，自动缓存刷新
    pub async fn get_access_token(&self) -> anyhow::Result<String> {
        {
            let cache = self.token_cache.read().await;
            // 提前 5 分钟刷新
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
            .get("https://qyapi.weixin.qq.com/cgi-bin/gettoken")
            .query(&[("corpid", &self.corp_id), ("corpsecret", &self.secret)])
            .send()
            .await
            .context("failed to fetch access_token")?;

        #[derive(Deserialize)]
        struct TokenResp {
            errcode: i32,
            errmsg: Option<String>,
            access_token: Option<String>,
            expires_in: Option<u64>,
        }

        let token_resp: TokenResp = resp
            .json()
            .await
            .context("failed to parse token response")?;
        if token_resp.errcode != 0 {
            bail!(
                "get access_token failed: errcode={} errmsg={:?}",
                token_resp.errcode,
                token_resp.errmsg
            );
        }

        let token = token_resp.access_token.unwrap_or_default();
        let expires_in = token_resp.expires_in.unwrap_or(7200);

        *cache = CachedToken {
            token: token.clone(),
            expires_at: Instant::now() + std::time::Duration::from_secs(expires_in),
        };

        tracing::info!(expires_in = expires_in, "access_token refreshed");
        Ok(token)
    }

    /// 发送文本消息给指定用户
    pub async fn send_text(&self, to_user: &str, content: &str) -> anyhow::Result<()> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/message/send?access_token={}",
            token
        );

        let body = serde_json::json!({
            "touser": to_user,
            "msgtype": "text",
            "agentid": self.agent_id.parse::<i32>().unwrap_or(0),
            "text": {
                "content": content
            },
            "safe": 0,
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("failed to send message")?;

        #[derive(Deserialize)]
        struct SendResp {
            errcode: i32,
            errmsg: Option<String>,
        }

        let send_resp: SendResp = resp.json().await.context("failed to parse send response")?;
        if send_resp.errcode != 0 {
            bail!(
                "send message failed: errcode={} errmsg={:?}",
                send_resp.errcode,
                send_resp.errmsg
            );
        }

        tracing::info!(to = %to_user, len = content.len(), "message sent");
        Ok(())
    }
}
