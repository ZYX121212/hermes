// crates/hermess-platform/src/adapters/discord.rs
// Discord Bot API 适配器（HTTP + 简单的 interaction webhook 接收）。

use async_trait::async_trait;

use crate::*;

pub struct DiscordConfig {
    pub bot_token: String,
    /// Discord API 版本
    pub api_version: String,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            api_version: "10".into(),
        }
    }
}

pub struct DiscordAdapter {
    config: DiscordConfig,
    http: reqwest::Client,
    tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    rx: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>>,
}

impl DiscordAdapter {
    pub fn new(config: DiscordConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            config,
            http: reqwest::Client::new(),
            tx,
            rx: parking_lot::Mutex::new(Some(rx)),
        }
    }

    fn api_url(&self) -> String {
        format!("https://discord.com/api/v{}", self.config.api_version)
    }

    /// 发送消息到 Discord channel。
    pub async fn send_channel_message(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
        buttons: Option<&ApprovalButtons>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "content": text,
        });

        if let Some(msg_id) = reply_to {
            body["message_reference"] = serde_json::json!({"message_id": msg_id});
        }

        if let Some(btns) = buttons {
            body["components"] = serde_json::json!([{
                "type": 1, // Action Row
                "components": [{
                    "type": 2, // Button
                    "style": 3, // Success (green)
                    "label": btns.approve_label,
                    "custom_id": format!("{}:approve", btns.action_id),
                }, {
                    "type": 2,
                    "style": 4, // Danger (red)
                    "label": btns.deny_label,
                    "custom_id": format!("{}:deny", btns.action_id),
                }]
            }]);
        }

        self.http
            .post(format!("{}/channels/{channel_id}/messages", self.api_url()))
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .json(&body)
            .send()
            .await?;

        Ok(())
    }

    /// 将 Discord Interaction 转换为 InboundMessage。
    /// 用于 integration via webhook endpoint。
    pub fn convert_interaction(&self, interaction: &serde_json::Value) -> Option<InboundMessage> {
        let typ = interaction["type"].as_i64()?;
        match typ {
            2 => {
                // Application Command (slash command)
                let data = &interaction["data"];
                let user = interaction.get("user")?;
                let msg = InboundMessage {
                    message_id: interaction["id"].as_str()?.to_string(),
                    user_id: user["id"].as_str()?.to_string(),
                    chat_id: interaction["channel_id"].as_str()?.to_string(),
                    text: data["options"].as_array().map_or(String::new(), |opts| {
                        opts.iter()
                            .map(|o| o["value"].as_str().unwrap_or("").to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    }),
                    kind: MessageKind::Command {
                        command: data["name"].as_str()?.to_string(),
                        args: String::new(),
                    },
                    platform: "discord".into(),
                    raw: interaction.clone(),
                };
                Some(msg)
            }
            3 => {
                // Message Component (button click)
                let user = interaction.get("user")?;
                let data_str = interaction["data"]["custom_id"].as_str()?;
                Some(InboundMessage {
                    message_id: interaction["id"].as_str()?.to_string(),
                    user_id: user["id"].as_str()?.to_string(),
                    chat_id: interaction["channel_id"].as_str()?.to_string(),
                    text: String::new(),
                    kind: MessageKind::Button {
                        callback_data: data_str.to_string(),
                    },
                    platform: "discord".into(),
                    raw: interaction.clone(),
                })
            }
            _ => None,
        }
    }

    /// 直接注入入站消息（用于 webhook-based 接收）。
    pub fn inject_message(&self, msg: InboundMessage) {
        let _ = self.tx.send(msg);
    }
}

#[async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn platform_name(&self) -> &str {
        "discord"
    }

    async fn start(&self) -> anyhow::Result<()> {
        tracing::info!("Discord adapter started");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.send_channel_message(
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
    fn test_convert_slash_command() {
        let adapter = DiscordAdapter::new(DiscordConfig::default());
        let interaction = serde_json::json!({
            "id": "int123",
            "type": 2,
            "channel_id": "ch456",
            "user": {"id": "user789", "username": "tester"},
            "data": {
                "name": "ask",
                "options": [{"name": "query", "value": "what is Rust?"}]
            }
        });
        let msg = adapter.convert_interaction(&interaction).unwrap();
        assert_eq!(msg.user_id, "user789");
        assert_eq!(msg.chat_id, "ch456");
        assert_eq!(msg.platform, "discord");
    }
}
