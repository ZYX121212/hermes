// axum HTTP 服务器 — 飞书 Bot + 通用 Chat API
use crate::feishu::client::FeishuClient;
use crate::session::SessionManager;
use agent_core::context::Context;
use agent_core::AgentEvent;
use agent_core::HermesAgent;
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
pub(crate) async fn run_agent_once(
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
