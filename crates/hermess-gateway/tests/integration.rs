use axum::body::Body;
use hermess_gateway::config::{
    ClassifierConfig, GatewayConfig, GatewaySection, OptimizerConfig, ShgConfig,
};
use hermess_gateway::feedback::FeedbackTracker;
use hermess_gateway::gateway::Gateway;
use hermess_gateway::models::*;
use hermess_gateway::registry::ModelRegistry;
use hermess_gateway::server;
use hermess_gateway::shg::ShgDetector;
use hermess_gateway::strategy::RouteStrategy;
use tower::ServiceExt;

/// Test the full SHG pipeline without a live LLM.
#[tokio::test]
async fn full_pipeline_shg_triggers() {
    let shg = ShgDetector {
        enabled: true,
        prompt_len_threshold: 200,
        patterns: vec!["formal proof".into()],
        force_model: Some("claude-opus-4-6".into()),
    };

    let result = shg.check("Give a formal proof of the theorem");
    assert_eq!(result, Some("claude-opus-4-6".into()));
}

#[test]
fn strategy_pipeline_cost_first() {
    let models = vec![
        ModelEntry {
            name: "cheap".into(),
            provider: "openai".into(),
            base_url: String::new(),
            api_key: "k".into(),
            cost_per_1m_input: 0.3,
            cost_per_1m_output: 0.6,
            capability: ModelCapability {
                reasoning: 0.4,
                coding: 0.5,
                creative: 0.3,
                knowledge: 0.5,
                speed_ms: 50,
            },
            tags: vec!["fast".into()],
        },
        ModelEntry {
            name: "smart".into(),
            provider: "anthropic".into(),
            base_url: String::new(),
            api_key: "k".into(),
            cost_per_1m_input: 15.0,
            cost_per_1m_output: 75.0,
            capability: ModelCapability {
                reasoning: 0.95,
                coding: 0.9,
                creative: 0.8,
                knowledge: 0.9,
                speed_ms: 2000,
            },
            tags: vec!["reasoning".into()],
        },
    ];

    // Low complexity -> cheapest model
    let cls = Classification {
        complexity: 0.2,
        is_short_hard: false,
        suggested_tags: vec![],
    };
    let fb = FeedbackTracker::new();
    let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models, &fb);
    assert_eq!(pick, Some("cheap".into()));

    // High complexity -> smart model (cheap fails min cap threshold)
    let cls = Classification {
        complexity: 0.9,
        is_short_hard: false,
        suggested_tags: vec![],
    };
    let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models, &fb);
    assert_eq!(pick, Some("smart".into()));
}

#[test]
fn registry_capability_lookup() {
    let mut reg = ModelRegistry::new();
    reg.add(ModelEntry {
        name: "m1".into(),
        provider: "openai".into(),
        base_url: String::new(),
        api_key: "k".into(),
        cost_per_1m_input: 1.0,
        cost_per_1m_output: 2.0,
        capability: ModelCapability {
            reasoning: 0.9,
            coding: 0.5,
            creative: 0.5,
            knowledge: 0.5,
            speed_ms: 100,
        },
        tags: vec!["reasoning".into()],
    });
    reg.add(ModelEntry {
        name: "m2".into(),
        provider: "openai".into(),
        base_url: String::new(),
        api_key: "k".into(),
        cost_per_1m_input: 0.1,
        cost_per_1m_output: 0.1,
        capability: ModelCapability {
            reasoning: 0.2,
            coding: 0.5,
            creative: 0.5,
            knowledge: 0.5,
            speed_ms: 50,
        },
        tags: vec!["fast".into()],
    });

    assert_eq!(reg.most_capable("reasoning").unwrap().name, "m1");
    assert_eq!(reg.cheapest().unwrap().name, "m2");
    assert_eq!(reg.fastest().unwrap().name, "m2");
}

// ── HTTP integration tests ───────────────────────────────────────

fn test_config() -> GatewayConfig {
    GatewayConfig {
        gateway: GatewaySection {
            listen: "0.0.0.0:0".into(),
            api_key: String::new(),
            default_mode: RouteMode::CostFirst,
            models: vec![],
            discovery: vec![],
            classifier: ClassifierConfig::default(),
            shg: ShgConfig {
                enabled: false,
                prompt_len_threshold: 500,
                hard_patterns: vec![],
                force_model: None,
            },
            optimizer: OptimizerConfig::default(),
        },
    }
}

#[tokio::test]
async fn http_health_returns_ok() {
    let gateway = Gateway::new(test_config(), "test", true).await;
    let router = server::build_router(gateway);

    let resp = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn http_models_returns_json() {
    let gateway = Gateway::new(test_config(), "test", true).await;
    let router = server::build_router(gateway);

    let resp = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    assert!(json["data"].is_array());
}

#[tokio::test]
async fn http_chat_invalid_model_returns_400() {
    let gateway = Gateway::new(test_config(), "test", true).await;
    let router = server::build_router(gateway);

    let resp = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"nonexistent","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_model");
}

#[tokio::test]
async fn http_chat_no_auth_required_when_no_key() {
    let gateway = Gateway::new(test_config(), "test", true).await;
    let router = server::build_router(gateway);

    let resp = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"auto","messages":[{"role":"user","content":"test"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should NOT be 401 since no api_key configured
    assert_ne!(resp.status(), 401);
}

#[tokio::test]
async fn http_embeddings_returns_not_implemented() {
    let gateway = Gateway::new(test_config(), "test", true).await;
    let router = server::build_router(gateway);

    let resp = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/embeddings")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"input":"test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 501);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "not_implemented");
}
