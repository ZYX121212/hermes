// axum HTTP 服务器 — 企业微信回调处理
use crate::session::SessionManager;
use crate::wechat::{client::WeChatClient, crypto, msg};
use agent_core::context::Context;
use agent_core::AgentEvent;
use agent_core::HermesAgent;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── App State ────────────────────────────────────────────────

pub struct AppState {
    pub wx_config: crate::wechat_config::WeChatConfig,
    pub wx_client: Arc<WeChatClient>,
    pub sessions: Arc<SessionManager>,
}

// ── Callback Query ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    msg_signature: String,
    timestamp: String,
    nonce: String,
    /// URL 验证时携带
    echostr: Option<String>,
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
    Router::new()
        .route("/wechat/callback", get(verify_url).post(handle_message))
        .route("/chat", post(handle_chat))
        .route("/health", get(health))
        .with_state(state)
}

// ── Handlers ─────────────────────────────────────────────────

/// GET /wechat/callback — URL 验证
async fn verify_url(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
) -> Result<String, StatusCode> {
    let echostr = query.echostr.as_deref().unwrap_or("");

    if !crypto::verify_signature(
        &state.wx_config.token,
        &query.timestamp,
        &query.nonce,
        echostr,
        &query.msg_signature,
    ) {
        tracing::warn!("URL verification signature mismatch");
        return Err(StatusCode::FORBIDDEN);
    }

    match crypto::decrypt_msg(echostr, &state.wx_config.encoding_aes_key) {
        Ok(plain) => {
            tracing::info!("URL verification succeeded");
            Ok(plain)
        }
        Err(e) => {
            tracing::error!(error = %e, "URL verification decryption failed");
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

/// POST /wechat/callback — 接收消息
async fn handle_message(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
    body: String,
) -> Response {
    // 尝试使用加密/明文两种方式解析
    let inner_xml = if !body.contains("<Encrypt>") {
        body.clone()
    } else {
        let encrypted = match msg::parse_encrypted_xml(&body) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(error = %e, "failed to parse encrypted XML");
                return (StatusCode::BAD_REQUEST, "invalid xml").into_response();
            }
        };

        if !crypto::verify_signature(
            &state.wx_config.token,
            &query.timestamp,
            &query.nonce,
            &encrypted.encrypt,
            &query.msg_signature,
        ) {
            tracing::warn!("message signature mismatch");
            return (StatusCode::FORBIDDEN, "signature mismatch").into_response();
        }

        match crypto::decrypt_msg(&encrypted.encrypt, &state.wx_config.encoding_aes_key) {
            Ok(xml) => xml,
            Err(e) => {
                tracing::error!(error = %e, "failed to decrypt message");
                return (StatusCode::BAD_REQUEST, "decrypt failed").into_response();
            }
        }
    };

    // 解析内层消息
    let inner = match msg::parse_message(&inner_xml) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse inner message");
            return (StatusCode::OK, "ok").into_response();
        }
    };

    let user_id = inner.from_user_name.clone();
    let content = inner.content.clone();

    tracing::info!(%user_id, %content, "received wechat message");

    let (reply, _errors) = run_agent_once(&state, &user_id, &content).await;

    match state.wx_client.send_text(&user_id, &reply).await {
        Ok(()) => tracing::info!(%user_id, len = reply.len(), "reply sent"),
        Err(e) => tracing::error!(error = %e, %user_id, "failed to send reply"),
    }

    (StatusCode::OK, "ok").into_response()
}

/// POST /chat — 通用 HTTP 对话接口
///
/// 请求: `{"user_id": "alice", "message": "帮我查天气"}`
/// 响应: `{"reply": "...", "errors": []}`
async fn handle_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    tracing::info!(user_id = %req.user_id, msg = %req.message, "chat request");

    let (reply, errors) = run_agent_once(&state, &req.user_id, &req.message).await;

    Json(ChatResponse { reply, errors })
}

/// 运行一次 agent 循环，返回 (reply, errors)
async fn run_agent_once(state: &AppState, user_id: &str, content: &str) -> (String, Vec<String>) {
    let (agent_arc, mut event_rx) = state.sessions.get_or_create(user_id);
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
            let msg = format!("Agent 执行错误: {:#}", e);
            tracing::error!(%user_id, %msg);
            errors.push(msg);
        }
        Err(e) => {
            let msg = format!("Agent panic: {:#}", e);
            tracing::error!(%user_id, %msg);
            errors.push(msg);
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

/// GET /health — 健康检查
async fn health() -> &'static str {
    "ok"
}
