use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[allow(clippy::enum_variant_names)]
pub enum RouteMode {
    CostFirst,
    QualityFirst,
    LatencyFirst,
}

impl std::str::FromStr for RouteMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cost-first" | "cost_first" => Ok(Self::CostFirst),
            "quality-first" | "quality_first" => Ok(Self::QualityFirst),
            "latency-first" | "latency_first" => Ok(Self::LatencyFirst),
            other => Err(format!("unknown route mode: {other}")),
        }
    }
}

impl std::fmt::Display for RouteMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CostFirst => write!(f, "cost-first"),
            Self::QualityFirst => write!(f, "quality-first"),
            Self::LatencyFirst => write!(f, "latency-first"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapability {
    pub reasoning: f64,
    pub coding: f64,
    pub creative: f64,
    pub knowledge: f64,
    pub speed_ms: u64,
}

impl Default for ModelCapability {
    fn default() -> Self {
        Self { reasoning: 0.5, coding: 0.5, creative: 0.5, knowledge: 0.5, speed_ms: 1000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub cost_per_1m_input: f64,
    pub cost_per_1m_output: f64,
    #[serde(default)]
    pub capability: ModelCapability,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouteTarget {
    Single(String),
    Decomposed { critical: String, regular: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    pub complexity: f64,
    pub is_short_hard: bool,
    pub suggested_tags: Vec<String>,
}

impl Default for Classification {
    fn default() -> Self {
        Self { complexity: 0.5, is_short_hard: false, suggested_tags: vec![] }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutingDecision {
    pub target: RouteTarget,
    pub reasoning: String,
}

// ── OpenAI-compatible request/response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<UsageData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecomposedPrompt {
    pub critical: String,
    pub regular: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GatewayOutput {
    Single {
        model: String,
        prompt: String,
    },
    Decomposed {
        critical_model: String,
        critical_prompt: String,
        regular_model: String,
        regular_prompt: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_route_mode_cost() {
        assert_eq!("cost-first".parse::<RouteMode>().unwrap(), RouteMode::CostFirst);
        assert_eq!("cost_first".parse::<RouteMode>().unwrap(), RouteMode::CostFirst);
    }

    #[test]
    fn parse_route_mode_quality() {
        assert_eq!("quality-first".parse::<RouteMode>().unwrap(), RouteMode::QualityFirst);
    }

    #[test]
    fn parse_route_mode_latency() {
        assert_eq!("latency-first".parse::<RouteMode>().unwrap(), RouteMode::LatencyFirst);
    }

    #[test]
    fn parse_route_mode_invalid() {
        assert!("garbage".parse::<RouteMode>().is_err());
    }

    #[test]
    fn route_mode_display() {
        assert_eq!(RouteMode::CostFirst.to_string(), "cost-first");
        assert_eq!(RouteMode::QualityFirst.to_string(), "quality-first");
        assert_eq!(RouteMode::LatencyFirst.to_string(), "latency-first");
    }
}
