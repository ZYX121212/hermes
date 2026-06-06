# Feishu Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace WeChat integration with Feishu WebSocket bot, provide same agent interaction via Feishu messaging.

**Architecture:** Follow existing WeChat pattern — FeishuClient (REST API + token cache) + FeishuBot (WebSocket event loop) → SessionManager → SmallHermesAgent. FeishuClient holds app_id/app_secret, manages tenant_access_token with RwLock cache. FeishuBot connects to Feishu's WebSocket gateway, dispatches im.message.receive_v1 events to the agent pipeline.

**Tech Stack:** reqwest (REST), tokio-tungstenite (WebSocket), serde_json (JSON events), existing axum server

---

### Task 1: Cargo.toml — 更新依赖

**Files:**
- Modify: `crates/hermess-web/Cargo.toml`

- [ ] **Step 1: 替换依赖**

移除 WeChat 专用依赖，添加 WebSocket 依赖。将整个文件替换为：

```toml
[package]
name = "hermess-web"
version = "0.1.0"
description = "HTTP daemon with Feishu bot integration and general chat API"
license = "MIT"
edition = "2021"

[[bin]]
name = "hermess-webd"
path = "src/main.rs"

[dependencies]
agent-core = { path = "../agent-core" }
hermess-agent = { path = "../hermess-agent", features = ["tui"] }
planner = { path = "../planner" }
scheduler = { path = "../scheduler" }
reflector = { path = "../reflector" }
evolution = { path = "../evolution" }
memory = { path = "../memory" }
llm = { path = "../llm" }
tools = { path = "../tools" }
tui = { path = "../tui" }
hermess-finance = { path = "../hermess-finance" }

clap = { version = "4", features = ["derive"] }
axum = "0.8"
tokio = { workspace = true, features = ["full"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "limit", "request-id"] }
serde.workspace = true
serde_json.workspace = true
toml.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
anyhow.workspace = true
async-trait.workspace = true
uuid.workspace = true
chrono.workspace = true
dashmap.workspace = true
parking_lot.workspace = true
reqwest.workspace = true
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
futures-util = "0.3"
```

- [ ] **Step 2: Build verify**

```bash
cargo check -p hermess-web 2>&1 | head -20
```

Expected: 编译错误（模块引用 wechat 尚未更新），但 Cargo.toml 自身无语法错误

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-web/Cargo.toml
git commit -m "chore(hermess-web): swap WeChat deps for tokio-tungstenite + futures-util"
```

---

### Task 2: feishu/event.rs — 事件类型定义

**Files:**
- Create: `crates/hermess-web/src/feishu/event.rs`

- [ ] **Step 1: 创建事件类型文件**

```rust
// crates/hermess-web/src/feishu/event.rs
// 飞书 WebSocket 事件类型定义
use serde::Deserialize;

/// WebSocket 推送事件的外层信封
#[derive(Debug, Deserialize)]
pub struct EventEnvelope {
    pub schema: String,
    pub header: EventHeader,
    pub event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct EventHeader {
    pub event_id: String,
    pub event_type: String,
    pub create_time: String,
    pub token: String,
    pub app_id: String,
}

/// im.message.receive_v1 事件体
#[derive(Debug, Deserialize)]
pub struct MessageReceiveEvent {
    pub sender: Sender,
    pub message: Message,
}

#[derive(Debug, Deserialize)]
pub struct Sender {
    pub sender_id: SenderId,
}

#[derive(Debug, Deserialize)]
pub struct SenderId {
    pub open_id: String,
    #[serde(default)]
    pub union_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub message_id: String,
    pub chat_id: String,
    pub chat_type: String,
    pub message_type: String,
    pub content: String,
}

/// 文本消息 content 字段的 JSON 结构
#[derive(Debug, Deserialize)]
pub struct TextContent {
    pub text: String,
}

/// 解析消息 content（JSON 字符串 → TextContent）
pub fn parse_text_content(content: &str) -> Option<String> {
    serde_json::from_str::<TextContent>(content)
        .ok()
        .map(|t| t.text.trim().to_string())
        .filter(|t| !t.is_empty())
}
```

- [ ] **Step 2: 添加单元测试**

在文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_envelope() {
        let json = r#"{"schema":"im.message.receive_v1","header":{"event_id":"ev_001","event_type":"im.message.receive_v1","create_time":"1700000000000","token":"t_001","app_id":"cli_a"},"event":{}}"#;
        let env: EventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.schema, "im.message.receive_v1");
        assert_eq!(env.header.event_id, "ev_001");
    }

    #[test]
    fn test_parse_text_content() {
        let content = r#"{"text":" 你好世界 "}"#;
        assert_eq!(parse_text_content(content), Some("你好世界".into()));

        assert_eq!(parse_text_content(r#"{"text":"  "#), None);
        assert_eq!(parse_text_content("not json"), None);
    }
}
```

- [ ] **Step 3: Run test**

```bash
cargo test -p hermess-web -- feishu::event 2>&1
```

Expected: 编译+测试通过

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-web/src/feishu/event.rs
git commit -m "feat(feishu): add WebSocket event type definitions"
```

---

### Task 3: feishu/client.rs — REST API 客户端

**Files:**
- Create: `crates/hermess-web/src/feishu/client.rs`

- [ ] **Step 1: 创建 API 客户端**

```rust
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
```

- [ ] **Step 2: 添加测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = FeishuClient::new("app_001".into(), "secret_001".into());
        assert_eq!(client.app_id(), "app_001");
    }
}
```

在文件末尾追加。

- [ ] **Step 3: Run test**

```bash
cargo test -p hermess-web -- feishu::client 2>&1
```

Expected: 编译+测试通过

- [ ] **Step 4: Commit**

```bash
git add crates/hermess-web/src/feishu/client.rs
git commit -m "feat(feishu): add REST API client with token management"
```

---

### Task 4: feishu/bot.rs — WebSocket 长连接 Bot

**Files:**
- Create: `crates/hermess-web/src/feishu/bot.rs`

- [ ] **Step 1: 创建 Bot 模块**

```rust
// crates/hermess-web/src/feishu/bot.rs
// 飞书 WebSocket 长连接 Bot — 事件接收 + agent 交互
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async;
use tungstenite::Message;

use crate::session::SessionManager;
use crate::wechat::client::FeishuClient;

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
                    // tungstenite auto-responds to Pong, but explicit handling for text ping frames
                    if let Ok(text) = String::from_utf8(data) {
                        tracing::debug!(ping = %text, "received ping, will be auto-ponged");
                    }
                }
                Ok(Message::Close(frame)) => {
                    tracing::warn!(?frame, "ws close frame received");
                    break;
                }
                Ok(_) => {} // binary etc, ignore
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
            // 非文本消息：回复提示
            let _ = self
                .client
                .reply_text(
                    &msg.message.message_id,
                    "暂不支持该消息类型，请发送文字。",
                )
                .await;
            return;
        }

        let content = match event::parse_text_content(&msg.message.content) {
            Some(c) => c,
            None => return,
        };

        let user_id = msg.sender.sender_id.open_id;
        tracing::info!(%user_id, %content, "received feishu message");

        let (reply, _errors) = run_agent_once(&self.sessions, &user_id, &content).await;

        if let Err(e) = self.client.reply_text(&msg.message.message_id, &reply).await {
            tracing::error!(error = %e, %user_id, "failed to send reply");
        }
    }
}

/// 指数退避重连延迟: 1s → 2s → 4s → 8s → 16s → 30s (cap)
fn reconnect_delay(count: u64) -> Duration {
    let secs = 1u64.saturating_pow(count.min(5) as u32).min(30);
    Duration::from_secs(secs)
}

/// 运行一次 agent 循环（复用 server.rs 中的逻辑）
async fn run_agent_once(
    sessions: &SessionManager,
    user_id: &str,
    content: &str,
) -> (String, Vec<String>) {
    use agent_core::context::Context;
    use agent_core::{AgentEvent, HermesAgent};

    let (agent_arc, mut event_rx) = sessions.get_or_create(user_id);
    let agent_guard = agent_arc.lock().await;
    let ctx = Context::new(Some(content.to_string()));

    let agent_clone = Arc::clone(&agent_arc);
    let handle = tokio::spawn(async move {
        let mut ag = agent_clone.lock().await;
        ag.run_loop(ctx).await
    });
    drop(agent_guard);

    let mut reply = String::new();
    let mut errors: Vec<String> = Vec::new();

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(AgentEvent::SummaryReady { summary }) => {
                        if !summary.is_empty() {
                            reply = summary;
                        }
                        break;
                    }
                    Some(AgentEvent::AgentError { message }) => {
                        errors.push(message);
                    }
                    Some(AgentEvent::AgentStopped) => break,
                    None => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(120)) => {
                errors.push("处理超时 (120s)".to_string());
                break;
            }
        }
    }

    match handle.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            errors.push(format!("Agent 执行错误: {:#}", e));
        }
        Err(e) => {
            errors.push(format!("Agent panic: {:#}", e));
        }
    }

    if reply.is_empty() && errors.is_empty() {
        reply = "任务已处理完成。".to_string();
    } else if reply.is_empty() {
        reply = format!("处理遇到错误:\n{}", errors.join("\n"));
    } else if !errors.is_empty() {
        reply = format!("{}\n\n⚠ 错误:\n{}", reply, errors.join("\n"));
    }

    (reply, errors)
}
```

- [ ] **Step 2: Build check**

```bash
cargo check -p hermess-web 2>&1 | head -30
```

Expected: 会有编译错误（`use crate::wechat::client::FeishuClient` 以及在 `bot.rs` 中引用 `crate::wechat`），我们将在后续 tasks 中修复

- [ ] **Step 3: Commit**

```bash
git add crates/hermess-web/src/feishu/bot.rs
git commit -m "feat(feishu): add WebSocket bot with event loop and agent integration"
```

---

### Task 5: feishu/mod.rs — 模块入口

**Files:**
- Create: `crates/hermess-web/src/feishu/mod.rs`

- [ ] **Step 1: 创建模块入口**

```rust
// crates/hermess-web/src/feishu/mod.rs
pub mod client;
pub mod bot;
pub mod event;
```

- [ ] **Step 2: Commit**

```bash
git add crates/hermess-web/src/feishu/mod.rs
git commit -m "feat(feishu): add module entry point"
```

---

### Task 6: lib.rs — 替换 WeChatConfig 为 FeishuConfig

**Files:**
- Modify: `crates/hermess-web/src/lib.rs`

- [ ] **Step 1: 替换配置类型**

将 `lib.rs` 内容替换为：

```rust
// crates/hermess-web/src/lib.rs
use serde::Deserialize;

pub mod server;
pub mod session;
pub mod feishu;

// ── Config types shared across the crate ─────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WebAppConfig {
    pub feishu: feishu_config::FeishuConfig,
    pub server: ServerConfig,
    pub learning_rate: f64,
    pub working_memory_size: usize,
    pub max_concurrency: usize,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub qdrant: QdrantConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub scorer: ScorerConfig,
    #[serde(default)]
    pub api_key: String,
}

impl WebAppConfig {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let interpolated = Self::interpolate_env(&raw);
        Ok(toml::from_str(&interpolated)?)
    }

    fn interpolate_env(raw: &str) -> String {
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' && chars.peek() == Some(&'{') {
                chars.next();
                let mut var = String::new();
                let mut default = String::new();
                let mut in_default = false;
                let mut found_close = false;
                for c in chars.by_ref() {
                    if c == ':' && !in_default {
                        in_default = true;
                    } else if c == '}' {
                        found_close = true;
                        break;
                    } else if in_default {
                        default.push(c);
                    } else {
                        var.push(c);
                    }
                }
                if found_close {
                    let val = std::env::var(&var).unwrap_or(default);
                    out.push_str(&val);
                } else {
                    out.push_str("${");
                    out.push_str(&var);
                    if in_default {
                        out.push(':');
                        out.push_str(&default);
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}

/// 飞书应用配置
pub mod feishu_config {
    use serde::Deserialize;

    #[derive(Debug, Clone, Deserialize)]
    pub struct FeishuConfig {
        pub app_id: String,
        pub app_secret: String,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8080 }

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub max_tokens: u32,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5-20251001".into(),
            max_tokens: 4096,
            api_key: String::new(),
            base_url: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QdrantConfig {
    #[serde(default = "default_qdrant_url")]
    pub url: String,
    #[serde(default = "default_collection")]
    pub collection: String,
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: default_qdrant_url(),
            collection: default_collection(),
            embedding_dim: default_embedding_dim(),
        }
    }
}

fn default_qdrant_url() -> String { "http://localhost:6334".into() }
fn default_collection() -> String { "hermes_memory".into() }
fn default_embedding_dim() -> usize { 1024 }

#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_search_endpoint")]
    pub endpoint: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self { api_key: None, endpoint: default_search_endpoint() }
    }
}

fn default_search_endpoint() -> String {
    "https://api.search.brave.com/res/v1/web/search".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScorerConfig {
    #[serde(default = "default_success_weight")]
    pub success_weight: f64,
    #[serde(default = "default_latency_weight")]
    pub latency_weight: f64,
    #[serde(default = "default_quality_weight")]
    pub quality_weight: f64,
    #[serde(default = "default_latency_target")]
    pub latency_target_ms: u64,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self {
            success_weight: default_success_weight(),
            latency_weight: default_latency_weight(),
            quality_weight: default_quality_weight(),
            latency_target_ms: default_latency_target(),
        }
    }
}

fn default_success_weight() -> f64 { 0.6 }
fn default_latency_weight() -> f64 { 0.2 }
fn default_quality_weight() -> f64 { 0.2 }
fn default_latency_target() -> u64 { 2000 }
```

- [ ] **Step 2: Commit**

```bash
git add crates/hermess-web/src/lib.rs
git commit -m "feat(feishu): replace WeChatConfig with FeishuConfig"
```

---

### Task 7: server.rs — 更新 AppState 并移除 WeChat 路由

**Files:**
- Modify: `crates/hermess-web/src/server.rs`

- [ ] **Step 1: 重写 server.rs**

```rust
// axum HTTP 服务器 — 飞书 Bot + 通用 Chat API
use crate::feishu::client::FeishuClient;
use crate::session::SessionManager;
use agent_core::context::Context;
use agent_core::AgentEvent;
use axum::middleware::{self, Next};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestId, RequestId, SetRequestIdLayer},
};
use uuid::Uuid;

#[derive(Clone, Default)]
struct MakeRequestUuid;

impl MakeRequestId for MakeRequestUuid {
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        let id = Uuid::new_v4().to_string().parse().ok()?;
        Some(RequestId::new(id))
    }
}

// ── App State ────────────────────────────────────────────────

pub struct AppState {
    pub feishu_client: Arc<FeishuClient>,
    pub sessions: Arc<SessionManager>,
    pub api_key: String,
}

// ── Chat API types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatRequest {
    user_id: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    reply: String,
    errors: Vec<String>,
}

// ── Router ───────────────────────────────────────────────────

pub fn build_router(state: Arc<AppState>) -> Router {
    let has_auth = !state.api_key.is_empty();
    if has_auth {
        tracing::info!("API key authentication enabled for /chat endpoint");
    } else {
        tracing::warn!("No api_key configured — /chat endpoint is OPEN to all requests");
    }
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware);

    Router::new()
        .route("/chat", post(handle_chat))
        .route("/health", get(health))
        .layer((
            SetRequestIdLayer::x_request_id(MakeRequestUuid),
            RequestBodyLimitLayer::new(4 * 1024 * 1024),
            CorsLayer::permissive(),
        ))
        .layer(auth_layer)
        .with_state(state)
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = req.uri().path();
    if path == "/health" {
        return Ok(next.run(req).await);
    }
    if state.api_key.is_empty() {
        return Ok(next.run(req).await);
    }
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(token) = auth_header.strip_prefix("Bearer ") {
        if token == state.api_key {
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

// ── Handlers ─────────────────────────────────────────────────

/// POST /chat — 通用 HTTP 对话接口
async fn handle_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    tracing::info!(user_id = %req.user_id, msg = %req.message, "chat request");

    let (reply, errors) = run_agent_once(&state.sessions, &req.user_id, &req.message).await;

    Json(ChatResponse { reply, errors })
}

async fn health() -> &'static str {
    "ok"
}

/// 运行一次 agent 循环（Bot 和 HTTP 端点共用）
async fn run_agent_once(
    sessions: &SessionManager,
    user_id: &str,
    content: &str,
) -> (String, Vec<String>) {
    let (agent_arc, mut event_rx) = sessions.get_or_create(user_id);
    let agent_guard = agent_arc.lock().await;
    let ctx = Context::new(Some(content.to_string()));

    let agent_clone = Arc::clone(&agent_arc);
    let handle = tokio::spawn(async move {
        let mut ag = agent_clone.lock().await;
        ag.run_loop(ctx).await
    });
    drop(agent_guard);

    let mut reply = String::new();
    let mut errors: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(AgentEvent::SummaryReady { summary }) => {
                        if !summary.is_empty() {
                            reply = summary;
                        }
                        break;
                    }
                    Some(AgentEvent::AgentError { message }) => {
                        errors.push(message);
                    }
                    Some(AgentEvent::AgentStopped) => break,
                    None => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                errors.push("处理超时 (120s)".to_string());
                break;
            }
        }
    }

    match handle.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            errors.push(format!("Agent 执行错误: {:#}", e));
        }
        Err(e) => {
            errors.push(format!("Agent panic: {:#}", e));
        }
    }

    if reply.is_empty() && errors.is_empty() {
        reply = "任务已处理完成。".to_string();
    } else if reply.is_empty() {
        reply = format!("处理遇到错误:\n{}", errors.join("\n"));
    } else if !errors.is_empty() {
        reply = format!("{}\n\n⚠ 错误:\n{}", reply, errors.join("\n"));
    }

    (reply, errors)
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/hermess-web/src/server.rs
git commit -m "feat(feishu): update AppState and remove WeChat callback routes"
```

---

### Task 8: main.rs — 替换为 FeishuClient + FeishuBot

**Files:**
- Modify: `crates/hermess-web/src/main.rs`

- [ ] **Step 1: 重写 main.rs**

```rust
// hermess-webd — 飞书接入的 Hermes Agent 守护进程
use std::str::FromStr;
use std::sync::Arc;

use clap::Parser;
use hermess_web::feishu::bot::FeishuBot;
use hermess_web::feishu::client::FeishuClient;
use hermess_web::session::SessionManager;

#[derive(Parser)]
#[command(name = "hermess-webd")]
struct Cli {
    #[arg(short, long, default_value = "config/feishu.toml")]
    config: String,
}

fn init_tracing() {
    use std::env;
    let use_json = env::var("LOG_FORMAT").map(|v| v == "json").unwrap_or(false);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if use_json {
        builder.json().init();
    } else {
        builder.init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let cfg = hermess_web::WebAppConfig::from_file(&cli.config)?;

    tracing::info!(
        "hermess-webd starting: provider={}, model={}, server={}:{}",
        cfg.llm.provider,
        cfg.llm.model,
        cfg.server.host,
        cfg.server.port,
    );

    // ── 共享资源 ──────────────────────────────────────────
    let memory: Arc<dyn agent_core::MemoryStore> = Arc::new(
        memory::VectorMemory::new(&memory::VectorMemoryConfig {
            url: cfg.qdrant.url.clone(),
            collection: cfg.qdrant.collection.clone(),
            embedding_dim: cfg.qdrant.embedding_dim,
        })
        .await?,
    );

    let llm: Arc<dyn llm::LlmAdapter> = match cfg.llm.provider.as_str() {
        "openai" | "deepseek" => {
            let key = if cfg.llm.api_key.is_empty() {
                std::env::var("OPENAI_API_KEY")
                    .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
                    .unwrap_or_default()
            } else {
                cfg.llm.api_key.clone()
            };
            let base_url = if cfg.llm.base_url.is_empty() {
                if cfg.llm.provider == "deepseek" {
                    "https://api.deepseek.com/v1".to_string()
                } else {
                    "https://api.openai.com/v1".to_string()
                }
            } else {
                cfg.llm.base_url.clone()
            };
            Arc::new(llm::OpenAIAdapter::new(&llm::OpenAIConfig {
                api_key: key,
                model: if cfg.llm.model.is_empty() && cfg.llm.provider == "deepseek" {
                    "deepseek-chat".into()
                } else {
                    cfg.llm.model.clone()
                },
                max_tokens: cfg.llm.max_tokens,
                base_url,
            }))
        }
        _ => {
            let key = if cfg.llm.api_key.is_empty() {
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
            } else {
                cfg.llm.api_key.clone()
            };
            Arc::new(llm::AnthropicAdapter::new(&llm::AnthropicConfig {
                api_key: key,
                model: cfg.llm.model.clone(),
                max_tokens: cfg.llm.max_tokens,
            }))
        }
    };

    let tools = Arc::new(tools::ToolRegistry::default());
    let danger_guard = Arc::new(tools::DangerGuard::new(
        tools::ConfirmationPolicy::from_str("ask").unwrap(),
        vec![],
    ));
    tools.register(Arc::new(tools::ReplyTool));
    tools.register(Arc::new(tools::BashTool::new(Arc::clone(&danger_guard))));
    tools.register(Arc::new(tools::ReadFileTool));
    tools.register(Arc::new(tools::WriteFileTool));
    tools.register(Arc::new(tools::WebSearchTool::new(&tools::SearchConfig {
        api_key: cfg.search.api_key.clone(),
        endpoint: cfg.search.endpoint.clone(),
    })));

    let finance_provider = hermess_finance::providers::defaults::build_finance_provider(
        hermess_finance::providers::defaults::FinanceProviderOptions {
            provider: std::env::var("HERMESS_FINANCE_PROVIDER").ok(),
            ftshare_url: std::env::var("HERMESS_FINANCE_URL").ok(),
            tushare_token: std::env::var("HERMESS_TUSHARE_TOKEN").ok(),
            allow_disable: true,
        },
    );
    tools.register(Arc::new(hermess_finance::tool::FinancialTool::new(
        finance_provider,
    )));

    let evolution = Arc::new(
        evolution::EvolutionEngine::load_from_file(
            ".hermes_web_evolution.json",
            cfg.learning_rate,
            Arc::clone(&memory),
        )
        .unwrap_or_else(|e| {
            let err_str = e.to_string();
            if err_str.contains("No such file") || err_str.contains("entity not found") {
                tracing::info!("No previous evolution state found, starting fresh");
            } else {
                tracing::warn!(error = %e, "Failed to load evolution state, starting fresh");
            }
            evolution::EvolutionEngine::new(cfg.learning_rate, Arc::clone(&memory))
        })
        .with_auto_save(".hermes_web_evolution.json"),
    );

    let evolution_handle = Arc::clone(&evolution);

    // ── 飞书 API 客户端 ────────────────────────────────────
    let feishu_client = FeishuClient::new(
        cfg.feishu.app_id.clone(),
        cfg.feishu.app_secret.clone(),
    );

    // ── 会话管理器 ─────────────────────────────────────────
    let sessions = Arc::new(SessionManager::new(
        Arc::clone(&evolution),
        Arc::clone(&llm) as Arc<dyn llm::LlmAdapter>,
        Arc::clone(&tools),
        cfg.max_concurrency,
        cfg.working_memory_size,
    ));
    sessions.clone().start_cleanup();

    // ── 启动飞书 Bot（WebSocket 长连接）───────────────────
    let bot = FeishuBot::new(Arc::clone(&feishu_client), Arc::clone(&sessions));
    tokio::spawn(async move { bot.run().await });

    // ── HTTP 服务器 ────────────────────────────────────────
    let api_key = if cfg.api_key.is_empty() {
        std::env::var("HERMESS_API_KEY").unwrap_or_default()
    } else {
        cfg.api_key.clone()
    };
    let state = Arc::new(hermess_web::server::AppState {
        feishu_client,
        sessions,
        api_key,
    });

    let router = hermess_web::server::build_router(state);
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("hermess-webd listening on http://{}", addr);

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down...");
        })
        .await?;

    if let Err(e) = evolution_handle.save_to_file(".hermes_web_evolution.json") {
        tracing::warn!(error = %e, "Failed to save evolution state");
    }

    Ok(())
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/hermess-web/src/main.rs
git commit -m "feat(feishu): replace WeChatClient with FeishuClient + FeishuBot spawn"
```

---

### Task 9: bot.rs — 修复 import 路径

**Files:**
- Modify: `crates/hermess-web/src/feishu/bot.rs`

- [ ] **Step 1: 修复 bot.rs 中的 import**

在 Task 4 创建的 `bot.rs` 中，有一行错误的 import：

```
use crate::wechat::client::FeishuClient;
```

改为：

```
use crate::feishu::client::FeishuClient;
```

同时移除 `bot.rs` 中重复的 `run_agent_once` 定义（因为它在 `server.rs` 中已经有定义）。改为使用 `super::server::run_agent_once` 的方式。实际上更简单的方式是直接在 `bot.rs` 中引用其所在 crate 的 server 模块。但由于 `run_agent_once` 现在是 `server.rs` 里的私有函数，我们直接在 `bot.rs` 中调用 `server::run_agent_once`。需要将其设为 `pub(crate)`。

先让 bot.rs 保持不变，通过 `crate::server` 来引用。`server.rs` 中的 `run_agent_once` 需要标识为 `pub(crate)`。

执行替换操作：

在 `bot.rs` 中，将：
```rust
use crate::wechat::client::FeishuClient;
```
改为：
```rust
use crate::feishu::client::FeishuClient;
use crate::server;
```

在 `bot.rs` 中，删除 `run_agent_once` 函数定义（整个函数），并将调用处：
```rust
        let (reply, _errors) = run_agent_once(&self.sessions, &user_id, &content).await;
```
改为：
```rust
        let (reply, _errors) = server::run_agent_once(&self.sessions, &user_id, &content).await;
```

同时在 `server.rs` 中，将 `run_agent_once` 的签名改为 `pub(crate)`：

```rust
pub(crate) async fn run_agent_once(
```

- [ ] **Step 2: Commit**

```bash
git add crates/hermess-web/src/feishu/bot.rs crates/hermess-web/src/server.rs
git commit -m "fix(feishu): fix import paths and share run_agent_once"
```

---

### Task 10: 清理 WeChat 旧代码 + 配置文件

**Files:**
- Delete: `crates/hermess-web/src/wechat/mod.rs`
- Delete: `crates/hermess-web/src/wechat/client.rs`
- Delete: `crates/hermess-web/src/wechat/crypto.rs`
- Delete: `crates/hermess-web/src/wechat/msg.rs`
- Create: `config/feishu.toml`
- Delete: `config/wechat.toml`

- [ ] **Step 1: 删除 WeChat 代码**

```bash
rm crates/hermess-web/src/wechat/mod.rs
rm crates/hermess-web/src/wechat/client.rs
rm crates/hermess-web/src/wechat/crypto.rs
rm crates/hermess-web/src/wechat/msg.rs
rmdir crates/hermess-web/src/wechat/
```

- [ ] **Step 2: 创建飞书配置文件**

```toml
# Hermes Web Daemon — 飞书配置
learning_rate = 0.1
working_memory_size = 100
max_concurrency = 10

[feishu]
app_id = "cli_..."              # ← 待填写：飞书应用 App ID
app_secret = "..."              # ← 待填写：飞书应用 App Secret

[server]
host = "0.0.0.0"
port = 8080

[llm]
provider = "deepseek"
model = "deepseek-chat"
max_tokens = 4096
api_key = "${DEEPSEEK_API_KEY}"

[qdrant]
url = "http://localhost:6334"
collection = "hermes_memory"
embedding_dim = 1024

[search]
endpoint = "https://api.search.brave.com/res/v1/web/search"

[scorer]
success_weight = 0.6
latency_weight = 0.2
quality_weight = 0.2
latency_target_ms = 2000
```

- [ ] **Step 3: 删除旧配置文件**

```bash
rm config/wechat.toml
```

- [ ] **Step 4: Commit**

```bash
git rm crates/hermess-web/src/wechat/mod.rs
git rm crates/hermess-web/src/wechat/client.rs
git rm crates/hermess-web/src/wechat/crypto.rs
git rm crates/hermess-web/src/wechat/msg.rs
git rm config/wechat.toml
git add config/feishu.toml
git commit -m "feat(feishu): remove WeChat code, add Feishu config template"
```

---

### Task 11: 编译验证 + 修复

**Files:**
- (视编译错误而定)

- [ ] **Step 1: 编译检查**

```bash
cargo check -p hermess-web 2>&1
```

Expected: 编译通过（0 errors）

- [ ] **Step 2: 运行所有测试**

```bash
cargo test -p hermess-web 2>&1
```

Expected: 所有测试通过

- [ ] **Step 3: 如果有编译错误**

根据错误信息修复：
- 缺少 imports → 补充 use 语句
- 类型不匹配 → 修正类型
- 模块引用错误 → 修正路径

然后重复 Step 1-2

- [ ] **Step 4: Commit (如有修复)**

```bash
git add -A && git commit -m "fix(feishu): compile error fixes"
```
