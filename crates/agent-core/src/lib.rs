// crates/agent-core/src/lib.rs
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod agent;
pub mod context;
pub mod runner;
pub mod session;

pub use agent::HermesAgent;
pub use session::SessionState;

/// A chunk of memory with its embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    pub id: Uuid,
    pub content: String,
    pub embedding: Vec<f32>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Importance score. Higher = more valuable to retain.
    /// Default: 1.0 (neutral). Failures/lessons score higher.
    #[serde(default = "default_importance")]
    pub importance: f64,
}

fn default_importance() -> f64 { 1.0 }

/// User input combined with environment snapshot and relevant history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_input: String,
    pub env_state: serde_json::Value,
    pub memory_ctx: Vec<MemoryChunk>,
    /// Multi-turn conversation history (user_input, assistant_response) pairs.
    #[serde(default)]
    pub conversation_history: Vec<(String, String)>,
    /// Recent evolution insights for feedback into planning.
    #[serde(default)]
    pub recent_insights: Vec<String>,
}

/// A single execution step within a plan.
#[derive(Debug, Clone)]
pub struct Step {
    pub id: Uuid,
    pub tool: String,
    pub args: serde_json::Value,
    pub depends: Vec<Uuid>,
    pub strategy: String,
    /// Fallback tool names to try if the primary tool fails after retries.
    pub tool_candidates: Vec<String>,
    /// Whether this step can be delegated to a sub-agent for parallel execution.
    pub delegable: bool,
}

/// Directed acyclic graph representing step dependencies.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    edges: Vec<(Uuid, Uuid)>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self { edges: vec![] }
    }

    pub fn add_edge(&mut self, from: Uuid, to: Uuid) {
        self.edges.push((from, to));
    }

    /// Returns steps grouped into topological layers.
    /// Each layer contains steps whose dependencies are all satisfied by previous layers.
    pub fn topological_layers(&self, steps: &[Step]) -> Vec<Vec<Step>> {
        use std::collections::{HashMap, VecDeque};
        let mut in_degree: HashMap<Uuid, usize> = HashMap::new();
        let mut adj: HashMap<Uuid, Vec<Uuid>> = HashMap::new();

        for step in steps {
            in_degree.entry(step.id).or_insert(0);
            adj.entry(step.id).or_default();
        }

        for (from, to) in &self.edges {
            *in_degree.entry(*to).or_insert(0) += 1;
            adj.entry(*from).or_default().push(*to);
        }

        let step_map: HashMap<Uuid, &Step> = steps.iter().map(|s| (s.id, s)).collect();
        let mut queue: VecDeque<Uuid> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut layers: Vec<Vec<Step>> = vec![];

        while !queue.is_empty() {
            let mut layer = vec![];
            let mut next = VecDeque::new();
            while let Some(id) = queue.pop_front() {
                if let Some(&step) = step_map.get(&id) {
                    layer.push(step.clone());
                }
                if let Some(neighbors) = adj.get(&id) {
                    for &next_id in neighbors {
                        let d = in_degree.get_mut(&next_id).unwrap();
                        *d -= 1;
                        if *d == 0 {
                            next.push_back(next_id);
                        }
                    }
                }
            }
            if !layer.is_empty() {
                layers.push(layer);
            }
            queue = next;
        }

        layers
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution plan: a set of steps arranged in a DAG.
#[derive(Debug, Clone)]
pub struct Plan {
    pub id: Uuid,
    pub steps: Vec<Step>,
    pub dag: DependencyGraph,
}

/// Output of a single executed step.
#[derive(Debug, Clone)]
pub struct StepOutput {
    pub step_id: Uuid,
    pub tool: String,
    pub success: bool,
    pub content: String,
    pub duration_ms: u64,
}

/// Result of executing an entire plan.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub plan_id: Uuid,
    pub outputs: Vec<StepOutput>,
    pub success: bool,
    pub duration_ms: u64,
    /// The original user task that triggered this execution.
    /// Used by the reflector to generate context-aware insights.
    pub user_input: Option<String>,
}

impl ExecutionResult {
    /// Build a semantic strategy ID from the tools used and success pattern.
    /// This ensures the evolution engine tracks meaningful patterns rather
    /// than ephemeral step UUIDs.
    pub fn strategy_id(&self) -> String {
        let tools: Vec<String> = self
            .outputs
            .iter()
            .map(|o| format!("{}({})", o.tool, if o.success { "ok" } else { "fail" }))
            .collect();
        if tools.is_empty() {
            return "empty".into();
        }
        tools.join("+")
    }
}

/// An insight produced by the reflector, fed into the evolution engine.
#[derive(Debug, Clone)]
pub struct Insight {
    pub strategy_id: String,
    /// Score in [-1.0, 1.0]: negative = failure, positive = success.
    pub score: f64,
    pub embedding: Vec<f32>,
    pub lesson: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSubPhase {
    CallingLlm,
    ParsingResponse,
    ExecutingTool,
    WaitingForInput,
    Idle,
}

/// Events emitted by agent components during a run loop iteration.
/// Consumed by TUI, web dashboard, or any observer.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    // ── Lifecycle ──
    AgentStarted {
        name: String,
    },
    AgentStopped,
    TurnStarted {
        turn: u64,
    },

    // ── Plan phase ──
    PlanPhaseStarted,
    /// A single token from LLM streaming output.
    PlanStreamingToken {
        token: String,
    },
    /// Plan parsing succeeded with this many steps.
    PlanReady {
        steps_count: usize,
    },
    /// First parse failed, retrying.
    PlanRetry,

    // ── Execute phase ──
    ExecutePhaseStarted {
        total_steps: usize,
    },
    /// A step is about to execute.
    StepStarted {
        step_id: Uuid,
        tool: String,
        layer: usize,
    },
    /// A step finished executing.
    StepCompleted {
        output: StepOutput,
    },
    /// Entire plan execution completed.
    ExecutePhaseComplete {
        all_success: bool,
        duration_ms: u64,
    },

    // ── Sub-agent delegation ──
    SubAgentStarted {
        task: String,
    },
    SubAgentCompleted {
        task: String,
        summary: String,
    },

    // ── Replan (when execution fails and needs retry with new plan) ──
    ReplanNeeded {
        reason: String,
        attempt: usize,
    },
    ReplanComplete {
        new_steps_count: usize,
    },

    // ── Reflect phase ──
    ReflectPhaseStarted,
    ReflectPhaseComplete {
        score: f64,
        lesson: String,
    },

    // ── Evolve phase ──
    EvolvePhaseStarted,
    EvolvePhaseComplete,

    // ── Summary ──
    /// A single token from the summary streaming output.
    SummaryStreamingToken {
        token: String,
    },
    SummaryReady {
        summary: String,
    },

    // ── Gateway ──
    /// Models discovered from the gateway at startup.
    GatewayModelsDiscovered {
        models: Vec<String>,
        gateway_url: String,
    },
    /// A routing decision was made by the gateway.
    GatewayRouteDecision {
        model: String,
        shg_triggered: bool,
        reason: String,
    },

    // ── Task tracking (kanban) ──
    TaskUpdated {
        task_id: String,
        title: String,
        status: TaskStatus,
    },

    // ── Thinking sub-phase ──
    ThinkingPhaseChanged {
        sub_phase: ThinkingSubPhase,
    },

    // ── Personality ──
    SetPersonality {
        name: String,
    },

    // ── Context compression ──
    CompressContext,

    // ── Checkpoint ──
    SaveCheckpoint,
    RollbackCheckpoint,

    // ── Session ──
    ResetSession,

    // ── Errors ──
    AgentError {
        message: String,
    },
}

/// Long-term memory storage abstraction.
/// Implementations include Qdrant-backed vector memory and in-memory mocks for testing.
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store or update a memory chunk.
    async fn upsert(&self, chunk: MemoryChunk) -> anyhow::Result<()>;
    /// Semantic search for the k most relevant chunks.
    async fn search(&self, query: &str, k: usize) -> anyhow::Result<Vec<MemoryChunk>>;
}
