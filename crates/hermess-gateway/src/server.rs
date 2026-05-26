use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Json, Response,
    },
    routing::{get, post},
    Router,
};
use futures::StreamExt;

use crate::gateway::Gateway;
use crate::models::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ErrorDetail,
    ErrorResponse, GatewayOutput, ModelInfo, ModelListResponse, UsageData,
};

pub struct AppState {
    pub gateway: Gateway,
}

pub fn build_router(gateway: Gateway) -> Router {
    let state = Arc::new(AppState { gateway });
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/embeddings", post(embeddings_handler))
        .with_state(state)
}

async fn health_handler() -> &'static str {
    "ok"
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

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    let mode = req.mode.as_deref().and_then(|m| m.parse().ok());

    let prompt = Gateway::extract_prompt(&req.messages);

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
        if req.stream {
            return handle_stream(adapter, prompt).await.into_response();
        }
        return handle_non_stream(adapter, prompt, &req.model)
            .await
            .into_response();
    }

    // Route via gateway pipeline
    let output = state.gateway.route(&prompt, mode).await;

    match output {
        GatewayOutput::Single { model, prompt } => {
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
            if req.stream {
                return handle_stream(adapter, prompt).await.into_response();
            }
            handle_non_stream(adapter, prompt, &model)
                .await
                .into_response()
        }
        GatewayOutput::Decomposed {
            critical_model,
            critical_prompt,
            regular_model,
            regular_prompt,
        } => {
            let crit_entry = state.gateway.lookup_model(&critical_model);
            let reg_entry = state.gateway.lookup_model(&regular_model);

            let (crit_result, reg_result) = tokio::join!(
                async {
                    if let Some(ref entry) = crit_entry {
                        let adapter = Gateway::build_adapter(entry);
                        if let Some(a) = adapter {
                            return a.complete(critical_prompt).await.ok();
                        }
                    }
                    None
                },
                async {
                    if let Some(ref entry) = reg_entry {
                        let adapter = Gateway::build_adapter(entry);
                        if let Some(a) = adapter {
                            return a.complete(regular_prompt).await.ok();
                        }
                    }
                    None
                },
            );

            let merged = crate::merger::ResultMerger::merge(
                crit_result.as_deref().unwrap_or(""),
                reg_result.as_deref().unwrap_or(""),
            );

            Json(ChatCompletionResponse {
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
            .into_response()
        }
    }
}

async fn handle_non_stream(
    adapter: Box<dyn llm::LlmAdapter>,
    prompt: String,
    model: &str,
) -> Result<Json<ChatCompletionResponse>, (StatusCode, Json<ErrorResponse>)> {
    match adapter.complete(prompt).await {
        Ok(text) => {
            let usage = adapter.last_usage();
            Ok(Json(ChatCompletionResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".into(),
                created: chrono::Utc::now().timestamp(),
                model: model.to_string(),
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
            }))
        }
        Err(e) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("Backend model error: {e:#}"),
        )),
    }
}

async fn handle_stream(
    adapter: Box<dyn llm::LlmAdapter>,
    prompt: String,
) -> Result<
    Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>>,
    (StatusCode, Json<ErrorResponse>),
> {
    match adapter.complete_stream(prompt).await {
        Ok(stream) => {
            let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
            let sse_stream = stream.map(move |chunk| match chunk {
                Ok(token) => {
                    let data = serde_json::json!({
                        "id": id,
                        "object": "chat.completion.chunk",
                        "created": chrono::Utc::now().timestamp(),
                        "choices": [{
                            "index": 0,
                            "delta": {"content": token},
                            "finish_reason": null
                        }]
                    });
                    Ok(Event::default()
                        .data(serde_json::to_string(&data).unwrap_or_default()))
                }
                Err(e) => {
                    let data = serde_json::json!({
                        "error": {"message": format!("{e:#}"), "type": "stream_error"}
                    });
                    Ok(Event::default()
                        .data(serde_json::to_string(&data).unwrap_or_default()))
                }
            });
            Ok(Sse::new(sse_stream))
        }
        Err(e) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            format!("Backend stream error: {e:#}"),
        )),
    }
}

fn api_error(status: StatusCode, error_type: &str, message: String) -> (StatusCode, Json<ErrorResponse>) {
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
