// crates/hermess-platform/src/adapters/slack.rs
// Slack Bot API 适配器（Web API + Events API）。
use async_trait::async_trait;

use crate::*;

pub struct SlackConfig {
    pub bot_token: String,
    pub signing_secret: Option<String>,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            signing_secret: None,
        }
    }
}

pub struct SlackAdapter {
    config: SlackConfig,
    http: reqwest::Client,
    tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    rx: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>>,
}

impl SlackAdapter {
    pub fn new(config: SlackConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            config,
            http: reqwest::Client::new(),
            tx,
            rx: parking_lot::Mutex::new(Some(rx)),
        }
    }

    /// 发送消息到 Slack channel。
    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
        buttons: Option<&ApprovalButtons>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::json!(ts);
        }

        if let Some(btns) = buttons {
            body["blocks"] = serde_json::json!([
                {"type": "section", "text": {"type": "mrkdwn", "text": text}},
                {"type": "actions", "elements": [
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": btns.approve_label},
                        "style": "primary",
                        "action_id": format!("{}:approve", btns.action_id)
                    },
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": btns.deny_label},
                        "style": "danger",
                        "action_id": format!("{}:deny", btns.action_id)
                    }
                ]}
            ]);
        }

        self.http
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .json(&body)
            .send()
            .await?;

        Ok(())
    }

    /// 将 Slack Event 转换为 InboundMessage。
    pub fn convert_event(&self, event: &serde_json::Value) -> Option<InboundMessage> {
        let event_type = event.get("type")?.as_str()?;

        match event_type {
            "message" => {
                // 忽略 bot 自己的消息
                if event.get("bot_id").is_some() || event.get("subtype").is_some() {
                    return None;
                }

                let text = event["text"].as_str()?.to_string();
                Some(InboundMessage {
                    message_id: event["ts"].as_str()?.to_string(),
                    user_id: event["user"].as_str()?.to_string(),
                    chat_id: event["channel"].as_str()?.to_string(),
                    text,
                    kind: MessageKind::Text,
                    platform: "slack".into(),
                    raw: event.clone(),
                })
            }
            "app_mention" => {
                let text = event["text"].as_str()?.to_string();
                Some(InboundMessage {
                    message_id: event["ts"].as_str()?.to_string(),
                    user_id: event["user"].as_str()?.to_string(),
                    chat_id: event["channel"].as_str()?.to_string(),
                    text,
                    kind: MessageKind::Text,
                    platform: "slack".into(),
                    raw: event.clone(),
                })
            }
            "block_actions" => {
                let actions = event["actions"].as_array()?;
                let action = actions.first()?;
                Some(InboundMessage {
                    message_id: event["trigger_id"].as_str()?.to_string(),
                    user_id: event["user"]["id"].as_str()?.to_string(),
                    chat_id: event["channel"]["id"].as_str()?.to_string(),
                    text: String::new(),
                    kind: MessageKind::Button {
                        callback_data: action["action_id"].as_str()?.to_string(),
                    },
                    platform: "slack".into(),
                    raw: event.clone(),
                })
            }
            _ => None,
        }
    }

    /// 直接注入消息（用于 webhook 接收）。
    pub fn inject_message(&self, msg: InboundMessage) {
        let _ = self.tx.send(msg);
    }
}

#[async_trait]
impl PlatformAdapter for SlackAdapter {
    fn platform_name(&self) -> &str {
        "slack"
    }

    async fn start(&self) -> anyhow::Result<()> {
        tracing::info!("Slack adapter started");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.post_message(
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
    fn test_slack_message_event() {
        let adapter = SlackAdapter::new(SlackConfig::default());
        let event = serde_json::json!({
            "type": "message",
            "user": "U123",
            "channel": "C456",
            "text": "hello slack",
            "ts": "1234567890.0001"
        });
        let msg = adapter.convert_event(&event).unwrap();
        assert_eq!(msg.user_id, "U123");
        assert_eq!(msg.chat_id, "C456");
        assert_eq!(msg.text, "hello slack");
        assert_eq!(msg.platform, "slack");
    }

    #[test]
    fn test_slack_ignore_bot_message() {
        let adapter = SlackAdapter::new(SlackConfig::default());
        let event = serde_json::json!({
            "type": "message",
            "bot_id": "B999",
            "user": "U123",
            "channel": "C456",
            "text": "bot message"
        });
        assert!(adapter.convert_event(&event).is_none());
    }
}
