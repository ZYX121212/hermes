// crates/hermess-platform/src/adapters/feishu.rs
// 飞书（Lark）开放平台 Bot API 适配器。
// 支持：消息发送（text/markdown/interactive card）、webhook 事件接收、
//       slash command、button callback。
//
// 参考: https://open.feishu.cn/document/server-docs/im-v1/message/create

use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

use crate::*;

pub struct FeishuConfig {
    /// 应用 App ID
    pub app_id: String,
    /// 应用 App Secret
    pub app_secret: String,
    /// Bot 自身的 open_id（用于过滤 bot 自己的消息）
    pub bot_open_id: Option<String>,
    /// 可选：自定义 API 域名（国际版用 open.larksuite.com）
    pub api_base: Option<String>,
    /// token 过期提前刷新时间（秒）
    pub token_refresh_margin_secs: u64,
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret: String::new(),
            bot_open_id: None,
            api_base: None,
            token_refresh_margin_secs: 300, // 飞书 token 有效期 2h，提前 5min 刷新
        }
    }
}

struct TokenCache {
    token: String,
    expires_at: Instant,
}

pub struct FeishuAdapter {
    config: FeishuConfig,
    http: reqwest::Client,
    tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    rx: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>>,
    token_cache: Mutex<Option<TokenCache>>,
}

impl FeishuAdapter {
    pub fn new(config: FeishuConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            tx,
            rx: parking_lot::Mutex::new(Some(rx)),
            token_cache: Mutex::new(None),
        }
    }

    fn api_base(&self) -> &str {
        self.config
            .api_base
            .as_deref()
            .unwrap_or("https://open.feishu.cn")
    }

    /// 获取 tenant_access_token（自动缓存和刷新）。
    async fn get_token(&self) -> anyhow::Result<String> {
        {
            let cache = self.token_cache.lock();
            if let Some(ref c) = *cache {
                if c.expires_at > Instant::now() {
                    return Ok(c.token.clone());
                }
            }
        }

        let resp: Value = self
            .http
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                self.api_base()
            ))
            .json(&serde_json::json!({
                "app_id": self.config.app_id,
                "app_secret": self.config.app_secret,
            }))
            .send()
            .await?
            .json()
            .await?;

        let token = resp["tenant_access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("飞书 token 获取失败: {}", resp))?
            .to_string();
        let expire_secs = resp["expire"].as_i64().unwrap_or(7200) as u64;
        let expires_at = Instant::now()
            + Duration::from_secs(
                expire_secs.saturating_sub(self.config.token_refresh_margin_secs),
            );

        let mut cache = self.token_cache.lock();
        *cache = Some(TokenCache {
            token: token.clone(),
            expires_at,
        });

        tracing::info!(expire_secs, "飞书 tenant_access_token 已刷新");
        Ok(token)
    }

    /// 发送消息到飞书群聊或私聊。
    /// - receive_id_type: "chat_id" / "open_id" / "user_id"
    pub async fn send_message(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        text: &str,
        reply_to: Option<&str>,
        buttons: Option<&ApprovalButtons>,
    ) -> anyhow::Result<()> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type={receive_id_type}",
            self.api_base()
        );

        let mut body = if let Some(btns) = buttons {
            // Interactive card with approval buttons
            serde_json::json!({
                "receive_id": receive_id,
                "msg_type": "interactive",
                "content": serde_json::json!({
                    "config": {"wide_screen_mode": true},
                    "header": {
                        "title": {"tag": "plain_text", "content": "Hermess 审批"},
                        "template": "blue"
                    },
                    "elements": [
                        {"tag": "markdown", "content": text},
                        {"tag": "action", "actions": [
                            {
                                "tag": "button",
                                "text": {"tag": "plain_text", "content": btns.approve_label},
                                "type": "primary",
                                "value": serde_json::json!({"action": "approve", "id": btns.action_id}).to_string()
                            },
                            {
                                "tag": "button",
                                "text": {"tag": "plain_text", "content": btns.deny_label},
                                "type": "danger",
                                "value": serde_json::json!({"action": "deny", "id": btns.action_id}).to_string()
                            }
                        ]}
                    ]
                }).to_string()
            })
        } else {
            serde_json::json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": serde_json::json!({"text": text}).to_string()
            })
        };

        if let Some(msg_id) = reply_to {
            body["root_id"] = serde_json::json!(msg_id);
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: Value = resp.json().await.unwrap_or_default();
            anyhow::bail!("飞书消息发送失败: {}", err);
        }

        Ok(())
    }

    /// 将飞书事件回调转换为 InboundMessage。
    pub fn convert_event(&self, event: &Value) -> Option<InboundMessage> {
        let header = event.get("header")?;
        let event_type = header.get("event_type")?.as_str()?;

        match event_type {
            "im.message.receive_v1" => {
                let ev = event.get("event")?;
                let msg = ev.get("message")?;
                let msg_type = msg.get("message_type")?.as_str()?;

                // 忽略 bot 自己的消息
                if let Some(ref bot_id) = self.config.bot_open_id {
                    let sender_open_id = ev
                        .get("sender")
                        .and_then(|s| s.get("sender_id"))
                        .and_then(|v| v.get("open_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if sender_open_id == bot_id.as_str() {
                        return None;
                    }
                }

                let sender = ev.get("sender")?;
                let sender_id = sender
                    .get("sender_id")
                    .and_then(|v| v.get("open_id"))
                    .or_else(|| sender.get("open_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                let chat_id = msg.get("chat_id")?.as_str()?;

                match msg_type {
                    "text" => {
                        let content: Value =
                            serde_json::from_str(msg.get("content")?.as_str()?).ok()?;
                        let text = content["text"].as_str()?.to_string();
                        Some(InboundMessage {
                            message_id: msg.get("message_id")?.as_str()?.to_string(),
                            user_id: sender_id.to_string(),
                            chat_id: chat_id.to_string(),
                            text,
                            kind: MessageKind::Text,
                            platform: "feishu".into(),
                            raw: event.clone(),
                        })
                    }
                    _ => {
                        // 其他消息类型（图片、文件等）
                        let content: Value =
                            serde_json::from_str(msg.get("content")?.as_str()?).ok()?;
                        let desc = content
                            .get("text")
                            .or_else(|| content.get("title"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some(InboundMessage {
                            message_id: msg.get("message_id")?.as_str()?.to_string(),
                            user_id: sender_id.to_string(),
                            chat_id: chat_id.to_string(),
                            text: desc,
                            kind: MessageKind::Unknown,
                            platform: "feishu".into(),
                            raw: event.clone(),
                        })
                    }
                }
            }
            "im.message.reaction.created_v1" => {
                let ev = event.get("event")?;
                Some(InboundMessage {
                    message_id: ev.get("message_id")?.as_str()?.to_string(),
                    user_id: ev.get("user_id")?.as_str()?.to_string(),
                    chat_id: ev.get("chat_id")?.as_str()?.to_string(),
                    text: String::new(),
                    kind: MessageKind::Unknown,
                    platform: "feishu".into(),
                    raw: event.clone(),
                })
            }
            "application.bot.menu_v6" => {
                // 机器人菜单事件
                let ev = event.get("event")?;
                let operators = ev.get("operator").or_else(|| ev.get("operators"));
                let operator_id = operators
                    .and_then(|o| o.get("operator_id"))
                    .or_else(|| operators.and_then(|o| o.get("open_id")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                Some(InboundMessage {
                    message_id: header.get("event_id")?.as_str()?.to_string(),
                    user_id: operator_id.to_string(),
                    chat_id: ev
                        .get("chat_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    text: serde_json::to_string(&ev).unwrap_or_default(),
                    kind: MessageKind::Command {
                        command: "bot_menu".into(),
                        args: String::new(),
                    },
                    platform: "feishu".into(),
                    raw: event.clone(),
                })
            }
            _ => None,
        }
    }

    /// 处理交互式卡片 button 回调。
    /// 飞书卡片 button 回调通过 card.action.trigger 事件传递。
    pub fn convert_card_action(&self, event: &Value) -> Option<InboundMessage> {
        let header = event.get("header")?;
        let event_type = header.get("event_type")?.as_str()?;
        if event_type != "card.action.trigger" {
            return None;
        }

        let ev = event.get("event")?;
        let action = ev.get("action")?;
        let value_str = action.get("value")?.as_str()?;
        let value: Value = serde_json::from_str(value_str).ok()?;

        let action_type = value["action"].as_str()?;
        let action_id = value["id"].as_str()?;
        let callback_data = format!("{action_id}:{action_type}");

        Some(InboundMessage {
            message_id: header.get("event_id")?.as_str()?.to_string(),
            user_id: ev.get("operator")?.get("open_id")?.as_str()?.to_string(),
            chat_id: ev
                .get("open_chat_id")
                .or_else(|| ev.get("chat_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            text: String::new(),
            kind: MessageKind::Button { callback_data },
            platform: "feishu".into(),
            raw: event.clone(),
        })
    }

    /// 直接注入入站消息（用于 webhook 接收端点）。
    pub fn inject_message(&self, msg: InboundMessage) {
        let _ = self.tx.send(msg);
    }
}

#[async_trait]
impl PlatformAdapter for FeishuAdapter {
    fn platform_name(&self) -> &str {
        "feishu"
    }

    async fn start(&self) -> anyhow::Result<()> {
        // 预热 token
        if !self.config.app_id.is_empty() {
            match self.get_token().await {
                Ok(_) => tracing::info!("Feishu adapter started (token acquired)"),
                Err(e) => tracing::warn!(error = %e, "Feishu token acquisition failed, will retry"),
            }
        } else {
            tracing::info!("Feishu adapter started (webhook-only mode)");
        }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.send_message(
            &msg.chat_id,
            "chat_id",
            &msg.text,
            msg.reply_to.as_deref(),
            msg.approval_buttons.as_ref(),
        )
        .await
    }

    fn inbound_rx(&self) -> tokio::sync::mpsc::UnboundedReceiver<InboundMessage> {
        self.rx.lock().take().unwrap_or_else(|| {
            let (_, rx) = tokio::sync::mpsc::unbounded_channel();
            rx
        })
    }

    fn supports_approval_buttons(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_text_message() {
        let adapter = FeishuAdapter::new(FeishuConfig::default());
        let event = serde_json::json!({
            "header": {
                "event_id": "evt_001",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_user123"
                    }
                },
                "message": {
                    "message_id": "om_msg456",
                    "chat_id": "oc_chat789",
                    "message_type": "text",
                    "content": "{\"text\":\"你好 飞书\"}"
                }
            }
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.user_id, "ou_user123");
        assert_eq!(msg.chat_id, "oc_chat789");
        assert_eq!(msg.text, "你好 飞书");
        assert_eq!(msg.platform, "feishu");
        assert_eq!(msg.kind, MessageKind::Text);
    }

    #[test]
    fn test_convert_card_action() {
        let adapter = FeishuAdapter::new(FeishuConfig::default());
        let event = serde_json::json!({
            "header": {
                "event_id": "evt_002",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_user123"
                },
                "open_chat_id": "oc_chat789",
                "action": {
                    "value": "{\"action\":\"approve\",\"id\":\"req-001\"}"
                }
            }
        });
        let msg = adapter.convert_card_action(&event).unwrap();
        assert_eq!(msg.user_id, "ou_user123");
        assert_eq!(msg.chat_id, "oc_chat789");
        match msg.kind {
            MessageKind::Button { callback_data } => {
                assert_eq!(callback_data, "req-001:approve");
            }
            _ => panic!("Expected Button kind"),
        }
    }

    #[test]
    fn test_ignore_bot_message() {
        let adapter = FeishuAdapter::new(FeishuConfig {
            bot_open_id: Some("bot_ou_001".into()),
            ..Default::default()
        });
        let event = serde_json::json!({
            "header": {
                "event_id": "evt_003",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "bot_ou_001"
                    }
                },
                "message": {
                    "message_id": "om_self",
                    "chat_id": "oc_chat",
                    "message_type": "text",
                    "content": "{\"text\":\"auto reply\"}"
                }
            }
        });
        assert!(adapter.convert_event(&event).is_none());
    }
}
