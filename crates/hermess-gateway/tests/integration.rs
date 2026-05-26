use hermess_gateway::models::*;
use hermess_gateway::registry::ModelRegistry;
use hermess_gateway::shg::ShgDetector;
use hermess_gateway::strategy::RouteStrategy;

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
    let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models);
    assert_eq!(pick, Some("cheap".into()));

    // High complexity -> smart model (cheap fails min cap threshold)
    let cls = Classification {
        complexity: 0.9,
        is_short_hard: false,
        suggested_tags: vec![],
    };
    let pick = RouteStrategy::decide(&RouteMode::CostFirst, &cls, &models);
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
