// crates/hermess-web/src/feishu/bot.rs
// 飞书 WebSocket 长连接 Bot — 事件接收 + agent 交互
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::feishu::client::FeishuClient;
use crate::server;
use crate::session::SessionManager;

use super::event::{self, EventEnvelope, MessageReceiveEvent};

pub struct FeishuBot {
    client: Arc<FeishuClient>,
    sessions: Arc<SessionManager>,
    state: RwLock<BotState>,
}

struct BotState {
    reconnect_count: u64,
}

impl FeishuBot {
    pub fn new(client: Arc<FeishuClient>, sessions: Arc<SessionManager>) -> Self {
        Self {
            client,
            sessions,
            state: RwLock::new(BotState { reconnect_count: 0 }),
        }
    }

    /// 启动 Bot，阻塞当前 task，包含自动重连
    pub async fn run(&self) {
        loop {
            match self.event_loop().await {
                Ok(()) => tracing::info!("bot event loop exited normally"),
                Err(e) => {
                    let mut st = self.state.write().await;
                    st.reconnect_count += 1;
                    let delay = reconnect_delay(st.reconnect_count);
                    tracing::error!(
                        error = %e,
                        reconnect = st.reconnect_count,
                        delay_secs = delay.as_secs(),
                        "bot disconnected, reconnecting..."
                    );
                    drop(st);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn event_loop(&self) -> anyhow::Result<()> {
        let ws_url = self.client.get_ws_url().await?;
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .context("failed to connect websocket")?;

        tracing::info!("bot connected to feishu ws gateway");
        self.state.write().await.reconnect_count = 0;

        let (_, mut read) = ws_stream.split();

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    self.handle_frame(&text).await;
                }
                Ok(Message::Ping(data)) => {
                    if let Ok(text) = String::from_utf8(data) {
                        tracing::debug!(ping = %text, "received ping frame");
                    }
                }
                Ok(Message::Close(frame)) => {
                    tracing::warn!(?frame, "ws close frame received");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(anyhow::anyhow!("ws read error: {e}"));
                }
            }
        }

        Ok(())
    }

    async fn handle_frame(&self, text: &str) {
        let envelope: EventEnvelope = match serde_json::from_str(text) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, raw = %text, "failed to parse event envelope");
                return;
            }
        };

        match envelope.schema.as_str() {
            "im.message.receive_v1" => {
                let msg: MessageReceiveEvent = match serde_json::from_value(envelope.event) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to parse message event");
                        return;
                    }
                };
                self.on_message(msg).await;
            }
            other => {
                tracing::debug!(schema = %other, "ignored event type");
            }
        }
    }

    async fn on_message(&self, msg: MessageReceiveEvent) {
        if msg.message.message_type != "text" {
            let _ = self
                .client
                .reply_text(&msg.message.message_id, "暂不支持该消息类型，请发送文字。")
                .await;
            return;
        }

        let content = match event::parse_text_content(&msg.message.content) {
            Some(c) => c,
            None => return,
        };

        let user_id = msg.sender.sender_id.open_id;
        tracing::info!(%user_id, %content, "received feishu message");

        let (reply, _errors) = server::run_agent_once(&self.sessions, &user_id, &content).await;

        if let Err(e) = self
            .client
            .reply_text(&msg.message.message_id, &reply)
            .await
        {
            tracing::error!(error = %e, %user_id, "failed to send reply");
        }
    }
}

/// 指数退避重连延迟: 1s → 2s → 4s → 8s → 16s → 30s (cap)
fn reconnect_delay(count: u64) -> Duration {
    let secs = 2u64.saturating_pow(count.min(4) as u32).min(30);
    Duration::from_secs(secs)
}
