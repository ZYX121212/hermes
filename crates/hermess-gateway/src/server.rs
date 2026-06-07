use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware::{self, Next},
    response::{
        sse::{Event, Sse},
        IntoResponse, Json, Response,
    },
    routing::{get, post},
    Router,
};
use futures::{stream, StreamExt};
use std::convert::Infallible;
use tokio::sync::Mutex;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestId, RequestId, SetRequestIdLayer},
};
use uuid::Uuid;

use crate::feedback::FeedbackTracker;
use crate::gateway::Gateway;
use crate::models::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ErrorDetail,
    ErrorResponse, GatewayOutput, ModelInfo, ModelListResponse, UsageData,
};

/// Generates UUID v4 request IDs
#[derive(Clone, Default)]
struct MakeRequestUuid;

impl MakeRequestId for MakeRequestUuid {
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        let id = Uuid::new_v4().to_string().parse().ok()?;
        Some(RequestId::new(id))
    }
}

pub struct AppState {
    pub gateway: Gateway,
}

pub fn build_router(gateway: Gateway) -> Router {
    let has_auth = !gateway.config.gateway.api_key.is_empty();
    if has_auth {
        tracing::info!("API key authentication enabled for gateway");
    } else {
        tracing::warn!("No API key configured — gateway endpoints are OPEN to all requests");
    }
    let state = Arc::new(AppState { gateway });
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware);

    Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/embeddings", post(embeddings_handler))
        .route("/v1/admin/memories", get(memories_handler))
        .route("/v1/admin/memory/attach", post(memory_attach_handler))
        .route("/v1/admin/memory/switch", post(memory_switch_handler))
        .layer((
            SetRequestIdLayer::x_request_id(MakeRequestUuid),
            RequestBodyLimitLayer::new(4 * 1024 * 1024), // 4MB body limit
            CorsLayer::permissive(),
        ))
        .layer(auth_layer)
        .with_state(state)
}

/// Authentication middleware — validates Bearer token if api_key is configured
async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if req.uri().path() == "/health" || req.uri().path() == "/metrics" {
        return Ok(next.run(req).await);
    }
    if state.gateway.config.gateway.api_key.is_empty() {
        return Ok(next.run(req).await);
    }
    let expected = &state.gateway.config.gateway.api_key;
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(token) = auth_header.strip_prefix("Bearer ") {
        if token == expected {
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime_secs = state.gateway.started_at.elapsed().as_secs();
    let model_count = state.gateway.list_models().len();
    let metrics = state.gateway.metrics.snapshot();
    let shg_enabled = state.gateway.config.gateway.shg.enabled;
    let shg_force_model = state
        .gateway
        .config
        .gateway
        .shg
        .force_model
        .as_deref()
        .unwrap_or("none");

    Json(serde_json::json!({
        "status": "ok",
        "instance": state.gateway.instance_name,
        "uptime_secs": uptime_secs,
        "model_count": model_count,
        "classifier_model": state.gateway.classifier_model,
        "shg_enabled": shg_enabled,
        "shg_force_model": shg_force_model,
        "default_mode": state.gateway.config.gateway.default_mode.to_string(),
        "skills_loaded": state.gateway.skill_set.len(),
        "skill_source": state.gateway.skill_set.source_dir.as_ref().map(|d| d.display().to_string()),
        "skill_names": state.gateway.skill_set.names(),
        "feedback_models_tracked": state.gateway.feedback.model_count(),
        "feedback_persisted": state.gateway.feedback.is_persisted(),
        "feedback_file": format!(".hermes_feedback_{}.json", state.gateway.instance_name),
        "feedback_snapshot": serde_json::to_value(state.gateway.feedback.snapshot()).unwrap_or_default(),
        "session_history_len": state.gateway.session_history.lock().await.len(),
        "metrics": {
            "total_requests": metrics.total_requests,
            "auto_routed": metrics.auto_routed,
            "shg_triggers": metrics.shg_triggers,
            "classifier_ok": metrics.classifier_ok,
            "classifier_timeout": metrics.classifier_timeout,
            "classifier_fallback": metrics.classifier_fallback,
            "upstream_errors": metrics.upstream_errors,
        }
    }))
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.gateway.metrics.snapshot())
}

async fn models_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let models: Vec<ModelInfo> = state
        .gateway
        .list_models()
        .iter()
        .map(|m| ModelInfo {
            id: m.name.clone(),
            object: "model".into(),
            created: chrono::Utc::now().timestamp(),
            owned_by: m.provider.clone(),
        })
        .collect();

    Json(ModelListResponse {
        object: "list".into(),
        data: models,
    })
}

/// Convert gateway ChatMessage to llm ChatMessage.
fn to_llm_messages(msgs: &[ChatMessage]) -> Vec<llm::ChatMessage> {
    msgs.iter()
        .map(|m| llm::ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect()
}

/// Build an llm::ChatCompletionRequest from the HTTP request parameters.
fn to_llm_request(req: &ChatCompletionRequest) -> llm::ChatCompletionRequest {
    llm::ChatCompletionRequest {
        messages: to_llm_messages(&req.messages),
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        top_p: req.top_p,
        stream: req.stream,
    }
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(mut req): Json<ChatCompletionRequest>,
) -> Response {
    state.gateway.metrics.inc_total();
    let mode = req.mode.as_deref().and_then(|m| m.parse().ok());

    // Inject skill context before extracting prompt for routing, so routing
    // sees the original user intent but the target model receives full context.
    let skill_prompt = state.gateway.skill_set.system_prompt();
    if !skill_prompt.is_empty() {
        req.messages.insert(
            0,
            ChatMessage {
                role: "system".into(),
                content: skill_prompt,
            },
        );
    }

    let prompt = Gateway::extract_prompt(&req.messages);
    let llm_req = to_llm_request(&req);

    // If model is not "auto", bypass routing and call directly
    if req.model != "auto" {
        let entry = match state.gateway.lookup_model(&req.model) {
            Some(e) => e,
            None => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_model",
                    format!(
                        "Model '{}' not found. Use 'auto' for routing or one of the registered models.",
                        req.model
                    ),
                )
                .into_response();
            }
        };
        let adapter = match Gateway::build_adapter(&entry) {
            Some(a) => a,
            None => {
                return api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "provider_error",
                    format!("Cannot build adapter for provider: {}", entry.provider),
                )
                .into_response();
            }
        };
        state.gateway.record_interaction(&prompt, &req.model).await;
        if req.stream {
            return handle_stream_chat(
                adapter,
                llm_req,
                &req.model,
                false,
                "",
                Arc::clone(&state.gateway.metrics),
                Arc::clone(&state.gateway.feedback),
            )
            .await;
        }
        return handle_non_stream_chat(
            adapter,
            llm_req,
            RouteCtx {
                model: &req.model,
                shg_triggered: false,
                route_reason: "",
                prompt: &prompt,
                metrics: Arc::clone(&state.gateway.metrics),
                feedback: Arc::clone(&state.gateway.feedback),
                session_history: &state.gateway.session_history,
            },
        )
        .await;
    }

    state.gateway.metrics.inc_auto();
    // Route via gateway pipeline
    let (output, reasoning) = state.gateway.route(&prompt, mode.clone()).await;
    let routed_model = match &output {
        GatewayOutput::Single { model, .. } => model.clone(),
        GatewayOutput::Decomposed {
            critical_model,
            regular_model,
            ..
        } => format!("{critical_model}+{regular_model}"),
    };
    let shg_triggered = reasoning.starts_with("SHG:");
    if shg_triggered {
        state.gateway.metrics.inc_shg();
    }
    if let Some(m) = mode.as_ref() {
        state.gateway.metrics.inc_strategy(m);
    }

    match output {
        GatewayOutput::Single { model, .. } => {
            let entry = match state.gateway.lookup_model(&model) {
                Some(e) => e,
                None => {
                    return api_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "routing_error",
                        format!("Routed model '{model}' not found in registry"),
                    )
                    .into_response();
                }
            };
            let adapter = match Gateway::build_adapter(&entry) {
                Some(a) => a,
                None => {
                    return api_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "provider_error",
                        format!("Cannot build adapter for provider: {}", entry.provider),
                    )
                    .into_response();
                }
            };
            state
                .gateway
                .record_interaction(&prompt, &routed_model)
                .await;
            if req.stream {
                return handle_stream_chat(
                    adapter,
                    llm_req,
                    &routed_model,
                    shg_triggered,
                    &reasoning,
                    Arc::clone(&state.gateway.metrics),
                    Arc::clone(&state.gateway.feedback),
                )
                .await;
            }
            handle_non_stream_chat(
                adapter,
                llm_req,
                RouteCtx {
                    model: &routed_model,
                    shg_triggered,
                    route_reason: &reasoning,
                    prompt: &prompt,
                    metrics: Arc::clone(&state.gateway.metrics),
                    feedback: Arc::clone(&state.gateway.feedback),
                    session_history: &state.gateway.session_history,
                },
            )
            .await
        }
        GatewayOutput::Decomposed {
            critical_model,
            critical_prompt,
            regular_model,
            regular_prompt,
        } => {
            state.gateway.metrics.inc_decomposer();
            let crit_entry = state.gateway.lookup_model(&critical_model);
            let reg_entry = state.gateway.lookup_model(&regular_model);

            let (crit_result, reg_result) = tokio::join!(
                async {
                    if let Some(ref entry) = crit_entry {
                        let adapter = Gateway::build_adapter(entry);
                        if let Some(a) = adapter {
                            let chat_req = llm::ChatCompletionRequest {
                                messages: vec![llm::ChatMessage {
                                    role: "user".into(),
                                    content: critical_prompt,
                                }],
                                max_tokens: req.max_tokens,
                                temperature: req.temperature,
                                top_p: req.top_p,
                                stream: false,
                            };
                            return a.complete_chat(chat_req).await.ok();
                        }
                    }
                    None
                },
                async {
                    if let Some(ref entry) = reg_entry {
                        let adapter = Gateway::build_adapter(entry);
                        if let Some(a) = adapter {
                            let chat_req = llm::ChatCompletionRequest {
                                messages: vec![llm::ChatMessage {
                                    role: "user".into(),
                                    content: regular_prompt,
                                }],
                                max_tokens: req.max_tokens,
                                temperature: req.temperature,
                                top_p: req.top_p,
                                stream: false,
                            };
                            return a.complete_chat(chat_req).await.ok();
                        }
                    }
                    None
                },
            );

            let merged = crate::merger::ResultMerger::merge(
                crit_result.as_deref().unwrap_or(""),
                reg_result.as_deref().unwrap_or(""),
            );

            let mut resp = Json(ChatCompletionResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".into(),
                created: chrono::Utc::now().timestamp(),
                model: format!("{critical_model}+{regular_model}"),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: merged,
                    },
                    finish_reason: "stop".into(),
                }],
                usage: None,
            })
            .into_response();
            resp.headers_mut().insert(
                "x-hermess-routed-model",
                axum::http::HeaderValue::from_str(&routed_model).unwrap(),
            );
            resp.headers_mut().insert(
                "x-hermess-route-reason",
                axum::http::HeaderValue::from_str(&reasoning).unwrap(),
            );
            resp.headers_mut().insert(
                "x-hermess-shg-triggered",
                axum::http::HeaderValue::from_str(&shg_triggered.to_string()).unwrap(),
            );
            resp
        }
    }
}

struct RouteCtx<'a> {
    model: &'a str,
    shg_triggered: bool,
    route_reason: &'a str,
    prompt: &'a str,
    metrics: Arc<crate::metrics::RouteMetrics>,
    feedback: Arc<crate::feedback::FeedbackTracker>,
    session_history: &'a Mutex<Vec<(String, String)>>,
}

async fn handle_non_stream_chat(
    adapter: Box<dyn llm::LlmAdapter>,
    llm_req: llm::ChatCompletionRequest,
    ctx: RouteCtx<'_>,
) -> Response {
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        adapter.complete_chat(llm_req),
    )
    .await;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(text)) => {
            ctx.feedback.record_success(ctx.model, elapsed_ms);
            // 记录最近 100 条对话
            {
                let mut hist = ctx.session_history.lock().await;
                hist.push((ctx.prompt.to_string(), text.clone()));
                if hist.len() > 100 {
                    hist.remove(0);
                }
            }
            let usage = adapter.last_usage();
            let mut resp = Json(ChatCompletionResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".into(),
                created: chrono::Utc::now().timestamp(),
                model: ctx.model.to_string(),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: text,
                    },
                    finish_reason: "stop".into(),
                }],
                usage: usage.map(|u| UsageData {
                    prompt_tokens: u.prompt_tokens,
                    completion_tokens: u.completion_tokens,
                    total_tokens: u.total_tokens,
                }),
            })
            .into_response();
            if !ctx.model.is_empty() && ctx.model != "auto" {
                resp.headers_mut().insert(
                    "x-hermess-routed-model",
                    axum::http::HeaderValue::from_str(ctx.model)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("")),
                );
            }
            if ctx.shg_triggered {
                resp.headers_mut().insert(
                    "x-hermess-shg-triggered",
                    axum::http::HeaderValue::from_static("true"),
                );
            }
            if !ctx.route_reason.is_empty() {
                resp.headers_mut().insert(
                    "x-hermess-route-reason",
                    axum::http::HeaderValue::from_str(ctx.route_reason)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("")),
                );
            }
            resp
        }
        Ok(Err(e)) => {
            ctx.feedback.record_failure(ctx.model, elapsed_ms);
            ctx.metrics.inc_upstream_error();
            api_error(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                format!("Backend model error: {e:#}"),
            )
            .into_response()
        }
        Err(_elapsed) => {
            ctx.feedback.record_failure(ctx.model, 120_000); // timeout at 120s
            ctx.metrics.inc_upstream_error();
            api_error(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                "Request to backend model timed out after 120s".into(),
            )
            .into_response()
        }
    }
}

async fn handle_stream_chat(
    adapter: Box<dyn llm::LlmAdapter>,
    llm_req: llm::ChatCompletionRequest,
    model: &str,
    shg_triggered: bool,
    route_reason: &str,
    metrics: Arc<crate::metrics::RouteMetrics>,
    feedback: Arc<crate::feedback::FeedbackTracker>,
) -> Response {
    let start = std::time::Instant::now();
    match adapter.complete_stream_chat(llm_req).await {
        Ok(stream) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            feedback.record_success(model, elapsed_ms);
            let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
            let model_owned = model.to_string();
            let reason_owned = route_reason.to_string();
            let sse_stream = stream.map(move |chunk| match chunk {
                Ok(token) => {
                    let data = serde_json::json!({
                        "id": id,
                        "object": "chat.completion.chunk",
                        "created": chrono::Utc::now().timestamp(),
                        "model": model_owned,
                        "choices": [{
                            "index": 0,
                            "delta": {"content": token},
                            "finish_reason": null
                        }]
                    });
                    Ok::<Event, Infallible>(
                        Event::default().data(serde_json::to_string(&data).unwrap_or_default()),
                    )
                }
                Err(e) => {
                    let data = serde_json::json!({
                        "error": {"message": format!("{e:#}"), "type": "stream_error"}
                    });
                    Ok::<Event, Infallible>(
                        Event::default().data(serde_json::to_string(&data).unwrap_or_default()),
                    )
                }
            });
            let done =
                stream::once(async { Ok::<Event, Infallible>(Event::default().data("[DONE]")) });
            let sse_stream = sse_stream.chain(done);

            let mut resp = Sse::new(sse_stream).into_response();
            if !model.is_empty() {
                resp.headers_mut().insert(
                    "x-hermess-routed-model",
                    axum::http::HeaderValue::from_str(model)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("")),
                );
            }
            if shg_triggered {
                resp.headers_mut().insert(
                    "x-hermess-shg-triggered",
                    axum::http::HeaderValue::from_static("true"),
                );
            }
            if !reason_owned.is_empty() {
                resp.headers_mut().insert(
                    "x-hermess-route-reason",
                    axum::http::HeaderValue::from_str(&reason_owned)
                        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("")),
                );
            }
            resp
        }
        Err(e) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            feedback.record_failure(model, elapsed_ms);
            metrics.inc_upstream_error();
            api_error(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                format!("Backend stream error: {e:#}"),
            )
            .into_response()
        }
    }
}

fn api_error(
    status: StatusCode,
    error_type: &str,
    message: String,
) -> (StatusCode, Json<ErrorResponse>) {
    tracing::warn!(%error_type, %message, "API error");
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                message,
                error_type: error_type.into(),
                code: None,
            },
        }),
    )
}

// ── Admin: runtime memory management ──────────────────────────

#[derive(serde::Deserialize)]
struct MemoryAction {
    instance: String,
}

async fn memories_handler() -> impl IntoResponse {
    let files = FeedbackTracker::list_available_files();
    Json(serde_json::json!({
        "memories": files,
        "count": files.len(),
    }))
}

async fn memory_attach_handler(
    State(state): State<Arc<AppState>>,
    Json(action): Json<MemoryAction>,
) -> impl IntoResponse {
    let file = format!(".hermes_feedback_{}.json", action.instance);
    match state.gateway.feedback.merge_from_file(&file) {
        Ok(count) => Json(serde_json::json!({
            "status": "ok",
            "action": "attach",
            "instance": action.instance,
            "models_merged": count,
        }))
        .into_response(),
        Err(e) => api_error(
            StatusCode::BAD_REQUEST,
            "memory_error",
            format!("无法合并记忆文件 {file}: {e}"),
        )
        .into_response(),
    }
}

async fn memory_switch_handler(
    State(state): State<Arc<AppState>>,
    Json(action): Json<MemoryAction>,
) -> impl IntoResponse {
    let file = format!(".hermes_feedback_{}.json", action.instance);
    if action.instance == state.gateway.instance_name {
        return api_error(
            StatusCode::BAD_REQUEST,
            "memory_error",
            format!("已经使用实例 '{}'，不能切换到自身", action.instance),
        )
        .into_response();
    }
    match state.gateway.feedback.replace_from_file(&file) {
        Ok(count) => Json(serde_json::json!({
            "status": "ok",
            "action": "switch",
            "instance": action.instance,
            "models_loaded": count,
        }))
        .into_response(),
        Err(e) => api_error(
            StatusCode::BAD_REQUEST,
            "memory_error",
            format!("无法加载记忆文件 {file}: {e}"),
        )
        .into_response(),
    }
}

async fn embeddings_handler(
    State(_state): State<Arc<AppState>>,
    Json(_req): Json<serde_json::Value>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: ErrorDetail {
                message: "Embeddings endpoint not yet implemented".into(),
                error_type: "not_implemented".into(),
                code: None,
            },
        }),
    )
}
