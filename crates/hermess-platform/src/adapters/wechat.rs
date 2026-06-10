// crates/hermess-platform/src/adapters/wechat.rs
// 企业微信（WeChat Work / 企业微信）Bot API 适配器。
// 支持：消息发送（text/markdown/template_card）、回调事件接收。
//
// 参考: https://developer.work.weixin.qq.com/document/path/90236

use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

use crate::*;

pub struct WechatConfig {
    /// 企业 ID
    pub corp_id: String,
    /// 应用 Secret
    pub corp_secret: String,
    /// 应用 Agent ID
    pub agent_id: u64,
    /// 可选：自定义 API 域名
    pub api_base: Option<String>,
    /// token 过期提前刷新时间（秒）
    pub token_refresh_margin_secs: u64,
}

impl Default for WechatConfig {
    fn default() -> Self {
        Self {
            corp_id: String::new(),
            corp_secret: String::new(),
            agent_id: 0,
            api_base: None,
            token_refresh_margin_secs: 300,
        }
    }
}

struct TokenCache {
    token: String,
    expires_at: Instant,
}

pub struct WechatAdapter {
    config: WechatConfig,
    http: reqwest::Client,
    tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    rx: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>>,
    token_cache: Mutex<Option<TokenCache>>,
}

impl WechatAdapter {
    pub fn new(config: WechatConfig) -> Self {
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
            .unwrap_or("https://qyapi.weixin.qq.com")
    }

    /// 获取 access_token（自动缓存和刷新）。
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
            .get(format!("{}/cgi-bin/gettoken", self.api_base()))
            .query(&[
                ("corpid", self.config.corp_id.as_str()),
                ("corpsecret", self.config.corp_secret.as_str()),
            ])
            .send()
            .await?
            .json()
            .await?;

        let errcode = resp["errcode"].as_i64().unwrap_or(-1);
        if errcode != 0 {
            anyhow::bail!(
                "企业微信 token 获取失败: {} (errcode={errcode})",
                resp["errmsg"].as_str().unwrap_or("unknown")
            );
        }

        let token = resp["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("企业微信 token 响应缺少 access_token: {resp}"))?
            .to_string();
        let expire_secs = resp["expires_in"].as_i64().unwrap_or(7200) as u64;
        let expires_at = Instant::now()
            + Duration::from_secs(
                expire_secs.saturating_sub(self.config.token_refresh_margin_secs),
            );

        let mut cache = self.token_cache.lock();
        *cache = Some(TokenCache {
            token: token.clone(),
            expires_at,
        });

        tracing::info!(expire_secs, "企业微信 access_token 已刷新");
        Ok(token)
    }

    /// 发送消息到企业微信用户/群聊。
    /// - touser: 成员 UserID / "@all" / 群聊 chat_id
    pub async fn send_message(
        &self,
        touser: &str,
        text: &str,
        reply_to: Option<&str>,
        buttons: Option<&ApprovalButtons>,
    ) -> anyhow::Result<()> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/cgi-bin/message/send?access_token={token}",
            self.api_base()
        );

        let mut body = if let Some(btns) = buttons {
            // template_card 交互卡片
            serde_json::json!({
                "touser": touser,
                "msgtype": "template_card",
                "agentid": self.config.agent_id,
                "template_card": {
                    "card_type": "text_notice",
                    "main_title": {
                        "title": "Hermess 审批",
                        "desc": text,
                    },
                    "card_action": {
                        "type": 1,
                        "url": "",
                    },
                    "button_selection": {
                        "question_key": "approval_action",
                        "title": "请选择操作",
                        "option_list": [
                            {
                                "id": format!("{}:approve", btns.action_id),
                                "text": btns.approve_label,
                            },
                            {
                                "id": format!("{}:deny", btns.action_id),
                                "text": btns.deny_label,
                            },
                        ],
                    },
                }
            })
        } else {
            let content = if let Some(_msg_id) = reply_to {
                format!("> 回复消息\n{text}")
            } else {
                text.to_string()
            };
            serde_json::json!({
                "touser": touser,
                "msgtype": "text",
                "agentid": self.config.agent_id,
                "text": {
                    "content": content,
                }
            })
        };

        // 支持 markdown 的消息类型检测
        if buttons.is_none() && (text.contains("**") || text.contains('#') || text.contains('`')) {
            let md_content = if reply_to.is_some() {
                format!("> 回复消息\n\n{text}")
            } else {
                text.to_string()
            };
            body = serde_json::json!({
                "touser": touser,
                "msgtype": "markdown",
                "agentid": self.config.agent_id,
                "markdown": {
                    "content": md_content,
                }
            });
        }

        let resp: Value = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let errcode = resp["errcode"].as_i64().unwrap_or(-1);
        if errcode != 0 {
            anyhow::bail!(
                "企业微信消息发送失败: {} (errcode={errcode})",
                resp["errmsg"].as_str().unwrap_or("unknown")
            );
        }

        Ok(())
    }

    /// 将企业微信回调事件转换为 InboundMessage。
    ///
    /// 企业微信回调支持 JSON 格式（设置回调 URL 时选择 JSON 模式）。
    /// 事件格式参考: <https://developer.work.weixin.qq.com/document/path/90240>
    pub fn convert_event(&self, event: &Value) -> Option<InboundMessage> {
        /// 企业微信 API 中 MsgId/CreateTime 为整数，其他字段为字符串。
        /// 此 helper 统一处理两种类型。
        fn val_str(v: &Value) -> Option<String> {
            match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }
        }

        let msg_type = event.get("MsgType")?.as_str()?;

        match msg_type {
            "text" => {
                let from_user = event.get("FromUserName")?.as_str()?;
                let chat_id = event
                    .get("ChatId")
                    .or_else(|| event.get("FromUserName"))
                    .and_then(|v| v.as_str())?;
                let content = event.get("Content").and_then(|v| v.as_str()).unwrap_or("");
                Some(InboundMessage {
                    message_id: val_str(event.get("MsgId")?)?,
                    user_id: from_user.to_string(),
                    chat_id: chat_id.to_string(),
                    text: content.to_string(),
                    kind: MessageKind::Text,
                    platform: "wechat".into(),
                    raw: event.clone(),
                })
            }
            "image" => {
                let from_user = event.get("FromUserName")?.as_str()?;
                let chat_id = event
                    .get("ChatId")
                    .or_else(|| event.get("FromUserName"))
                    .and_then(|v| v.as_str())?;
                let pic_url = event.get("PicUrl").and_then(|v| v.as_str()).unwrap_or("");
                Some(InboundMessage {
                    message_id: val_str(event.get("MsgId")?)?,
                    user_id: from_user.to_string(),
                    chat_id: chat_id.to_string(),
                    text: String::new(),
                    kind: MessageKind::Image {
                        url: pic_url.to_string(),
                        caption: None,
                    },
                    platform: "wechat".into(),
                    raw: event.clone(),
                })
            }
            "voice" => {
                let from_user = event.get("FromUserName")?.as_str()?;
                let chat_id = event
                    .get("ChatId")
                    .or_else(|| event.get("FromUserName"))
                    .and_then(|v| v.as_str())?;
                let recognition = event
                    .get("Recognition")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(InboundMessage {
                    message_id: val_str(event.get("MsgId")?)?,
                    user_id: from_user.to_string(),
                    chat_id: chat_id.to_string(),
                    text: recognition.clone().unwrap_or_default(),
                    kind: MessageKind::Voice {
                        url: String::new(),
                        transcription: recognition,
                    },
                    platform: "wechat".into(),
                    raw: event.clone(),
                })
            }
            "event" => {
                let event_type = event.get("Event")?.as_str()?;
                match event_type {
                    "click" | "view" => {
                        let from_user = event.get("FromUserName")?.as_str()?;
                        let event_key =
                            event.get("EventKey").and_then(|v| v.as_str()).unwrap_or("");
                        Some(InboundMessage {
                            message_id: format!(
                                "{}_{event_type}",
                                val_str(event.get("CreateTime")?)?
                            ),
                            user_id: from_user.to_string(),
                            chat_id: from_user.to_string(),
                            text: event_key.to_string(),
                            kind: MessageKind::Button {
                                callback_data: event_key.to_string(),
                            },
                            platform: "wechat".into(),
                            raw: event.clone(),
                        })
                    }
                    "subscribe" | "enter_agent" => {
                        let from_user = event.get("FromUserName")?.as_str()?;
                        Some(InboundMessage {
                            message_id: format!(
                                "{}_{event_type}",
                                val_str(event.get("CreateTime")?)?
                            ),
                            user_id: from_user.to_string(),
                            chat_id: from_user.to_string(),
                            text: String::new(),
                            kind: MessageKind::Command {
                                command: event_type.to_string(),
                                args: String::new(),
                            },
                            platform: "wechat".into(),
                            raw: event.clone(),
                        })
                    }
                    "template_card_event" => {
                        // 模板卡片按钮点击事件
                        let from_user = event.get("FromUserName")?.as_str()?;
                        let chat_id = event
                            .get("ChatId")
                            .or_else(|| event.get("FromUserName"))
                            .and_then(|v| v.as_str())?;
                        let first_item = event
                            .get("SelectedItems")
                            .and_then(|v| v.get("SelectedItem"))
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.first());
                        let selected = first_item
                            .and_then(|item| {
                                item.get("OptionId").and_then(|v| v.as_str()).or_else(|| {
                                    item.get("OptionIds")
                                        .and_then(|ids| ids.as_array())
                                        .and_then(|ids| ids.first())
                                        .and_then(|id| id.as_str())
                                })
                            })
                            .unwrap_or("");
                        Some(InboundMessage {
                            message_id: format!("tmpl_{}", val_str(event.get("CreateTime")?)?),
                            user_id: from_user.to_string(),
                            chat_id: chat_id.to_string(),
                            text: String::new(),
                            kind: MessageKind::Button {
                                callback_data: selected.to_string(),
                            },
                            platform: "wechat".into(),
                            raw: event.clone(),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 直接注入入站消息（用于 webhook 接收端点）。
    pub fn inject_message(&self, msg: InboundMessage) {
        let _ = self.tx.send(msg);
    }
}

#[async_trait]
impl PlatformAdapter for WechatAdapter {
    fn platform_name(&self) -> &str {
        "wechat"
    }

    async fn start(&self) -> anyhow::Result<()> {
        if !self.config.corp_id.is_empty() {
            match self.get_token().await {
                Ok(_) => tracing::info!("Wechat adapter started (token acquired)"),
                Err(e) => tracing::warn!(error = %e, "Wechat token acquisition failed, will retry"),
            }
        } else {
            tracing::info!("Wechat adapter started (webhook-only mode)");
        }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.send_message(
            &msg.chat_id,
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
        let adapter = WechatAdapter::new(WechatConfig::default());
        let event = serde_json::json!({
            "ToUserName": "wx_hermess",
            "FromUserName": "user_zhangsan",
            "CreateTime": "1717200000",
            "MsgType": "text",
            "Content": "你好 企业微信",
            "MsgId": "msg_001",
            "AgentID": 1000002
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.user_id, "user_zhangsan");
        assert_eq!(msg.text, "你好 企业微信");
        assert_eq!(msg.platform, "wechat");
        assert_eq!(msg.kind, MessageKind::Text);
    }

    #[test]
    fn test_convert_text_message_with_integer_msgid() {
        // 真实企业微信 API 中 MsgId 和 CreateTime 均为整数
        let adapter = WechatAdapter::new(WechatConfig::default());
        let event = serde_json::json!({
            "ToUserName": "wx_hermess",
            "FromUserName": "user_lisi",
            "CreateTime": 1717200000,
            "MsgType": "text",
            "Content": "整数 MsgId 测试",
            "MsgId": 1234567890,
            "AgentID": 1000002
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.user_id, "user_lisi");
        assert_eq!(msg.message_id, "1234567890");
        assert_eq!(msg.text, "整数 MsgId 测试");
    }

    #[test]
    fn test_convert_click_event() {
        let adapter = WechatAdapter::new(WechatConfig::default());
        let event = serde_json::json!({
            "ToUserName": "wx_hermess",
            "FromUserName": "user_zhangsan",
            "CreateTime": "1717200000",
            "MsgType": "event",
            "Event": "click",
            "EventKey": "menu_approve"
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.user_id, "user_zhangsan");
        match msg.kind {
            MessageKind::Button { callback_data } => {
                assert_eq!(callback_data, "menu_approve");
            }
            _ => panic!("Expected Button kind"),
        }
    }

    #[test]
    fn test_convert_template_card_event() {
        let adapter = WechatAdapter::new(WechatConfig::default());
        let event = serde_json::json!({
            "ToUserName": "wx_hermess",
            "FromUserName": "user_zhangsan",
            "CreateTime": "1717200000",
            "MsgType": "event",
            "Event": "template_card_event",
            "ChatId": "chat_456",
            "SelectedItems": {
                "SelectedItem": [
                    {
                        "QuestionKey": "approval_action",
                        "OptionIds": ["req-001:approve"]
                    }
                ]
            }
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.user_id, "user_zhangsan");
        assert_eq!(msg.chat_id, "chat_456");
        match msg.kind {
            MessageKind::Button { callback_data } => {
                assert_eq!(callback_data, "req-001:approve");
            }
            _ => panic!("Expected Button kind"),
        }
    }

    #[test]
    fn test_convert_voice_message() {
        let adapter = WechatAdapter::new(WechatConfig::default());
        let event = serde_json::json!({
            "ToUserName": "wx_hermess",
            "FromUserName": "user_zhangsan",
            "CreateTime": "1717200000",
            "MsgType": "voice",
            "MsgId": "msg_003",
            "Recognition": "帮我查一下订单状态"
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.text, "帮我查一下订单状态");
        match msg.kind {
            MessageKind::Voice {
                ref transcription, ..
            } => {
                assert_eq!(transcription.as_deref(), Some("帮我查一下订单状态"));
            }
            _ => panic!("Expected Voice kind"),
        }
    }
}
