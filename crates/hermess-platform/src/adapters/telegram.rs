// crates/hermess-platform/src/adapters/telegram.rs
// Telegram Bot API 适配器。
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::*;

pub struct TelegramConfig {
    pub bot_token: String,
    /// 轮询间隔（秒）
    pub poll_interval_secs: u64,
    /// 代理 URL（可选）
    pub api_base: Option<String>,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            poll_interval_secs: 2,
            api_base: None,
        }
    }
}

pub struct TelegramAdapter {
    config: TelegramConfig,
    http: reqwest::Client,
    tx: tokio::sync::mpsc::UnboundedSender<InboundMessage>,
    rx: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>>,
    running: parking_lot::Mutex<bool>,
    last_offset: Mutex<i64>,
}

impl TelegramAdapter {
    pub fn new(config: TelegramConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            tx,
            rx: parking_lot::Mutex::new(Some(rx)),
            running: parking_lot::Mutex::new(false),
            last_offset: Mutex::new(0),
        }
    }

    fn api_url(&self, method: &str) -> String {
        let base = self
            .config
            .api_base
            .clone()
            .unwrap_or_else(|| "https://api.telegram.org".into());
        format!("{}/bot{}/{}", base, self.config.bot_token, method)
    }

    async fn get_updates(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        let offset = *self.last_offset.lock().await;
        let url = self.api_url("getUpdates");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "offset": offset,
                "timeout": 30,
                "allowed_updates": ["message", "callback_query"]
            }))
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        let updates: Vec<serde_json::Value> = json["result"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(updates)
    }

    async fn send_telegram_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
        buttons: Option<&ApprovalButtons>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown",
        });

        if let Some(msg_id) = reply_to {
            if let Ok(id) = msg_id.parse::<i64>() {
                body["reply_to_message_id"] = serde_json::json!(id);
            }
        }

        if let Some(btns) = buttons {
            body["reply_markup"] = serde_json::json!({
                "inline_keyboard": [[
                    {
                        "text": btns.approve_label,
                        "callback_data": format!("{}:approve", btns.action_id)
                    },
                    {
                        "text": btns.deny_label,
                        "callback_data": format!("{}:deny", btns.action_id)
                    }
                ]]
            });
        }

        self.http
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await?;

        Ok(())
    }

    fn convert_message(&self, msg: &serde_json::Value) -> Option<InboundMessage> {
        let message = msg.get("message")?;
        let from = message.get("from")?;
        let chat = message.get("chat")?;
        let msg_id = message["message_id"].as_i64()?;

        let text = message["text"].as_str().unwrap_or("").to_string();

        Some(InboundMessage {
            message_id: msg_id.to_string(),
            user_id: from["id"].to_string(),
            chat_id: chat["id"].to_string(),
            text,
            kind: MessageKind::Text,
            platform: "telegram".into(),
            raw: message.clone(),
        })
    }

    fn convert_callback(&self, cb: &serde_json::Value) -> Option<InboundMessage> {
        let from = cb.get("from")?;
        let msg = cb.get("message")?;
        let chat = msg.get("chat")?;
        let data = cb["data"].as_str()?;

        Some(InboundMessage {
            message_id: cb["id"].to_string(),
            user_id: from["id"].to_string(),
            chat_id: chat["id"].to_string(),
            text: String::new(),
            kind: MessageKind::Button {
                callback_data: data.to_string(),
            },
            platform: "telegram".into(),
            raw: cb.clone(),
        })
    }
}

#[async_trait]
impl PlatformAdapter for TelegramAdapter {
    fn platform_name(&self) -> &str {
        "telegram"
    }

    async fn start(&self) -> anyhow::Result<()> {
        let mut running = self.running.lock();
        if *running {
            return Ok(());
        }
        *running = true;
        drop(running);

        tracing::info!("Telegram adapter started (polling every {}s)", self.config.poll_interval_secs);
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        *self.running.lock() = false;
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.send_telegram_message(
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

/// Poll Telegram for new messages (should be spawned as a background task).
pub async fn telegram_poll_loop(adapter: Arc<TelegramAdapter>) {
    loop {
        if !*adapter.running.lock() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        match adapter.get_updates().await {
            Ok(updates) => {
                for update in &updates {
                    if let Some(msg) = adapter
                        .convert_message(update)
                        .or_else(|| adapter.convert_callback(update))
                    {
                        let update_id = update["update_id"].as_i64().unwrap_or(0);
                        {
                            let mut offset = adapter.last_offset.lock().await;
                            *offset = update_id + 1;
                        }
                        if adapter.tx.send(msg).is_err() {
                            tracing::warn!("Telegram inbound channel closed");
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Telegram poll error: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }

        tokio::time::sleep(Duration::from_secs(adapter.config.poll_interval_secs)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_telegram_message() {
        let cfg = TelegramConfig {
            bot_token: "test".into(),
            ..Default::default()
        };
        let adapter = TelegramAdapter::new(cfg);
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 100,
                "from": {"id": 42, "first_name": "User"},
                "chat": {"id": 42, "type": "private"},
                "text": "hello world"
            }
        });

        let msg = adapter.convert_message(&update).unwrap();
        assert_eq!(msg.user_id, "42");
        assert_eq!(msg.chat_id, "42");
        assert_eq!(msg.text, "hello world");
        assert_eq!(msg.kind, MessageKind::Text);
        assert_eq!(msg.platform, "telegram");
    }

    #[test]
    fn test_convert_telegram_callback() {
        let cfg = TelegramConfig {
            bot_token: "test".into(),
            ..Default::default()
        };
        let adapter = TelegramAdapter::new(cfg);
        let update = serde_json::json!({
            "update_id": 2,
            "callback_query": {
                "id": "cb1",
                "from": {"id": 42, "first_name": "User"},
                "message": {"message_id": 100, "chat": {"id": 42, "type": "private"}},
                "data": "action1:approve"
            }
        });

        let msg = adapter.convert_callback(&update["callback_query"]).unwrap();
        assert_eq!(msg.kind, MessageKind::Button { callback_data: "action1:approve".into() });
    }
}
