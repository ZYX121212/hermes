# 🦀 Small Hermes Agent

**详细实现架构文档**

*Rust · Self-Evolution · 4,127 Lines*

| **项目** | **说明** |
| --- | --- |
| 项目名称 | Small Hermes Agent |
| 实现语言 | Rust (Edition 2021) |
| 总代码规模 | 约 4,127 行（不含测试和注释） |
| 核心框架 | Tokio async runtime |
| 文档版本 | v1.0  —  初始实现规范 |


---

# 1  项目概述
Small Hermes Agent 是一个用 Rust 实现的极简自进化 AI Agent。它的核心理念是"最小可进化系统"——足够小，一名工程师可以完整阅读；足够完整，具备真正的自我进化能力；足够快，可以在生产环境高速运转。

## 1.1  设计哲学
- 最小化：每一行代码都必须赚到自己的位置，严格控制在 ~4,000 行以内

- 可进化：Agent 每次执行后自动学习，持续优化自身策略权重

- 类型安全：用 Rust 的类型系统在编译期消除大量运行时错误

- 并发友好：基于 Tokio 的异步架构，支持数百工具调用并发执行

- 可观测：全链路 tracing，每一步执行都有完整的日志和指标

## 1.2  五步自进化循环
Agent 的所有行为都被组织在一个五步反馈循环中：

| **步骤** | **名称** | **职责** | **关键模块** |
| --- | --- | --- | --- |
| 01 | Observe | 感知环境，采集用户输入和环境状态 | env_observer, io_adapter |
| 02 | Plan | 将目标分解为可执行的子任务序列 | planner, llm_adapter |
| 03 | Execute | 并发调用工具执行各子任务 | executor, tool_registry, scheduler |
| 04 | Reflect | 评估执行结果，归因错误，生成 Insight | reflector, scorer |
| 05 | Evolve | 根据 Insight 更新策略权重，写入长期记忆 | evolution_engine, memory |

*💡 每次循环结束后，Agent 的策略权重会被更新——无需人工干预，系统自动变得更聪明。*


---

# 2  目录结构
项目采用 Rust workspace 结构，每个核心模块是一个独立的 crate，通过 trait 对象解耦。

```text
hermes-agent/
├── Cargo.toml               # workspace 根配置
├── Cargo.lock
├── README.md
├── config/
│   ├── default.toml         # 默认配置（learning_rate, memory 等）
│   └── production.toml
├── crates/
│   ├── agent-core/          # 主循环、生命周期、Context (~420 行)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── agent.rs     # HermesAgent trait 定义
│   │       ├── context.rs   # Context、StopSignal
│   │       └── runner.rs    # run_loop 默认实现
│   │
│   ├── evolution/           # 进化引擎 (~680 行)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── engine.rs    # EvolutionEngine 主体
│   │       ├── insight.rs   # Insight 数据结构
│   │       ├── scorer.rs    # 结果评分器
│   │       └── weight.rs    # 策略权重管理
│   │
│   ├── planner/             # 任务规划 (~350 行)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── planner.rs   # LLM 驱动的任务拆解
│   │       ├── plan.rs      # Plan、Step 数据结构
│   │       └── dependency.rs# 步骤依赖图（DAG）
│   │
│   ├── memory/              # 记忆系统 (~480 行)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── working.rs   # 短期工作记忆（Ring buffer）
│   │       ├── vector.rs    # 长期向量记忆（Qdrant）
│   │       └── embedding.rs # 文本向量化
│   │
│   ├── tools/               # 工具注册与调用 (~310 行)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── registry.rs  # ToolRegistry、动态注册
│   │       ├── caller.rs    # 统一调用接口
│   │       └── builtin/     # 内置工具（web_search, bash, …）
│   │
│   ├── reflector/           # 反思评估 (~290 行)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── reflector.rs # 评估逻辑主体
│   │       └── attribution.rs# 错误归因算法
│   │
│   ├── llm/                 # 大模型适配 (~380 行)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── adapter.rs   # LlmAdapter trait
│   │       ├── anthropic.rs # Claude API 实现
│   │       ├── openai.rs    # OpenAI API 实现
│   │       └── stream.rs    # 流式响应处理
│   │
│   └── scheduler/           # 异步调度 (~260 行)
│       └── src/
│           ├── lib.rs
│           ├── scheduler.rs # 任务调度器
│           └── concurrency.rs# 并发控制（Semaphore）
│
├── src/
│   └── main.rs              # CLI 入口（~80 行）
│
└── tests/
├── integration/         # 集成测试
└── fixtures/            # 测试数据
```

---

# 3  依赖配置 (Cargo.toml)
## 3.1  Workspace 根配置
```text
# Cargo.toml (workspace root)
[workspace]
members = [
"crates/agent-core",
"crates/evolution",
"crates/planner",
"crates/memory",
"crates/tools",
"crates/reflector",
"crates/llm",
"crates/scheduler",
]
resolver = "2"

[workspace.dependencies]
# Async runtime
tokio          = { version = "1", features = ["full"] }
async-trait    = "0.1"

# Serialization
serde          = { version = "1", features = ["derive"] }
serde_json     = "1"
toml           = "0.8"

# HTTP client
reqwest        = { version = "0.12", features = ["json", "stream"] }

# Concurrency primitives
dashmap        = "6"
parking_lot    = "0.12"
arc-swap       = "1"

# Vector DB client
qdrant-client  = "1"

# Observability
tracing        = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Error handling
anyhow         = "1"
thiserror      = "1"

# CLI
clap           = { version = "4", features = ["derive"] }

# Utilities
uuid           = { version = "1", features = ["v4"] }
chrono         = { version = "0.4", features = ["serde"] }
futures        = "0.3"
bytes          = "1"
```

---

# 4  核心 Trait 与数据结构
## 4.1  HermesAgent — 主 trait
所有智能体行为的核心契约，定义在 crates/agent-core/src/agent.rs。

```rust
// crates/agent-core/src/agent.rs
use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
pub trait HermesAgent: Send + Sync + 'static {
// ── 五步核心行为 ────────────────────────────────
async fn observe(&self, ctx: &Context) -> Result<Observation>;
async fn plan(&self, obs: Observation) -> Result<Plan>;
async fn execute(&self, plan: Plan) -> Result<ExecutionResult>;
async fn reflect(&self, result: &ExecutionResult) -> Result<Insight>;
async fn evolve(&mut self, insight: Insight) -> Result<()>;

// ── 默认实现：主循环 ────────────────────────────
async fn run_loop(&mut self, ctx: Context) -> Result<()> {
loop {
let obs     = self.observe(&ctx).await?;
let plan    = self.plan(obs).await?;
let result  = self.execute(plan).await?;
let insight = self.reflect(&result).await?;
self.evolve(insight).await?;
if ctx.should_stop() { break; }
}
Ok(())
}
}
```
## 4.2  核心数据结构
```rust
// crates/agent-core/src/lib.rs

/// 用户输入 + 环境快照
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Observation {
pub id:        uuid::Uuid,
pub timestamp: chrono::DateTime<chrono::Utc>,
pub user_input: String,
pub env_state: serde_json::Value,   // 环境元数据
pub memory_ctx: Vec<MemoryChunk>,   // 相关历史记忆
}

/// 执行计划（有向无环图）
#[derive(Debug, Clone)]
pub struct Plan {
pub id:    uuid::Uuid,
pub steps: Vec<Step>,
pub dag:   DependencyGraph,          // 步骤依赖关系
}

/// 单个执行步骤
#[derive(Debug, Clone)]
pub struct Step {
pub id:       uuid::Uuid,
pub tool:     String,               // 工具名称
pub args:     serde_json::Value,    // 工具参数
pub depends:  Vec<uuid::Uuid>,      // 依赖的步骤 ID
pub strategy: String,               // 策略标签（用于权重更新）
}

/// 执行结果
#[derive(Debug, Clone)]
pub struct ExecutionResult {
pub plan_id:    uuid::Uuid,
pub outputs:    Vec<StepOutput>,
pub success:    bool,
pub duration_ms: u64,
}

/// 反思后产生的洞见（喂给进化引擎）
#[derive(Debug, Clone)]
pub struct Insight {
pub strategy_id: String,
pub score:       f64,               // [-1.0, 1.0]：负=失败，正=成功
pub embedding:   Vec<f32>,          // 语义向量（用于长期记忆）
pub lesson:      String,            // 自然语言总结
}
```

---

# 5  各模块详细实现
| **5.1  进化引擎 (evolution_engine) — ~680 行** |
| --- |

进化引擎是 Hermes 的核心差异化模块。它负责根据每轮执行的 Insight，无锁地更新策略权重，并将经验写入长期向量记忆。

### 关键设计决策
- 使用 DashMap（无锁并发 HashMap）存储策略权重，支持多线程同时更新

- 学习率使用 AtomicF64，无需 Mutex 即可线程安全地读写

- 长期记忆写入与权重更新解耦，写入失败不影响主流程

```rust
// crates/evolution/src/engine.rs
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::Result;

pub struct EvolutionEngine {
/// 策略 ID -> 权重分数（无锁并发 HashMap）
strategy_weights: Arc<DashMap<String, f64>>,
/// 长期向量记忆（Qdrant）
memory_store:     Arc<VectorMemory>,
/// 学习率（原子浮点，f64 bits 转 u64 存储）
learning_rate_bits: AtomicU64,
/// 历史 Insight 数量（用于自适应学习率衰减）
insight_count:    AtomicU64,
}

impl EvolutionEngine {
pub fn new(lr: f64, memory: Arc<VectorMemory>) -> Self {
Self {
strategy_weights:   Arc::new(DashMap::new()),
memory_store:       memory,
learning_rate_bits: AtomicU64::new(lr.to_bits()),
insight_count:      AtomicU64::new(0),
}
}

/// 核心更新逻辑
pub async fn update(&self, insight: Insight) -> Result<()> {
// 1. 读取当前学习率
let lr   = f64::from_bits(
self.learning_rate_bits.load(Ordering::Relaxed));
let n    = self.insight_count.fetch_add(1, Ordering::Relaxed);
// 自适应衰减：lr_t = lr_0 / sqrt(n + 1)
let lr_t = lr / ((n + 1) as f64).sqrt();
let delta = insight.score * lr_t;

// 2. 无锁更新策略权重
self.strategy_weights
.entry(insight.strategy_id.clone())
.and_modify(│w│ *w = (*w + delta).clamp(-10.0, 10.0))
.or_insert(delta);

// 3. 异步写入长期记忆（失败不阻断主流程）
let store   = Arc::clone(&self.memory_store);
let emb_insight = insight.clone();
tokio::spawn(async move {
if let Err(e) = store.upsert(emb_insight.into()).await {
tracing::warn!("memory upsert failed: {e}");
}
});
Ok(())
}

/// 查询最优策略（规划时使用）
pub fn best_strategy(&self, candidates: &[&str]) -> Option<String> {
candidates.iter()
.filter_map(│s│ {
self.strategy_weights.get(*s)
.map(│w│ (s.to_string(), *w))
})
.max_by(│a, b│ a.1.partial_cmp(&b.1).unwrap())
.map(│(s, _)│ s)
}
}
```
| **5.2  任务规划器 (planner) — ~350 行** |
| --- |

Planner 接收 Observation，通过 LLM 将目标分解为步骤序列，并构建依赖关系 DAG（有向无环图）。

```rust
// crates/planner/src/planner.rs

pub struct Planner {
llm:       Arc<dyn LlmAdapter>,
evolution: Arc<EvolutionEngine>,  // 用于策略权重查询
}

impl Planner {
pub async fn plan(&self, obs: Observation) -> Result<Plan> {
// 1. 构造规划 Prompt（注入历史记忆 + 最优策略偏好）
let prompt = self.build_prompt(&obs);

// 2. 调用 LLM 生成步骤 JSON
let raw = self.llm.complete(prompt).await?;
let steps: Vec<StepSpec> = serde_json::from_str(&raw)?;

// 3. 构建依赖 DAG，检测环路
let dag = DependencyGraph::from_specs(&steps)?;

// 4. 为每个步骤选择最优策略
let steps = steps.into_iter().map(│s│ Step {
id:       uuid::Uuid::new_v4(),
tool:     s.tool,
args:     s.args,
depends:  s.depends.into_iter().map(│_│ uuid::Uuid::new_v4()).collect(),
strategy: self.evolution.best_strategy(&s.candidates)
.unwrap_or_else(││ "default".into()),
}).collect();

Ok(Plan { id: uuid::Uuid::new_v4(), steps, dag })
}

fn build_prompt(&self, obs: &Observation) -> String {
// 注入：用户目标 + 相关历史记忆 + 可用工具列表
// 要求 LLM 返回 JSON 格式的步骤数组
format!(/* ... */)
}
}
```
| **5.3  记忆系统 (memory) — ~480 行** |
| --- |

记忆系统分为两层：短期工作记忆（Ring buffer，驻内存）和长期向量记忆（Qdrant，持久化）。

```rust
// crates/memory/src/working.rs
/// 短期工作记忆：固定容量环形缓冲区
pub struct WorkingMemory {
buffer:   parking_lot::RwLock<VecDeque<MemoryChunk>>,
capacity: usize,
}

impl WorkingMemory {
pub fn push(&self, chunk: MemoryChunk) {
let mut buf = self.buffer.write();
if buf.len() >= self.capacity { buf.pop_front(); }
buf.push_back(chunk);
}
pub fn recent(&self, n: usize) -> Vec<MemoryChunk> {
let buf = self.buffer.read();
buf.iter().rev().take(n).cloned().collect()
}
}

// crates/memory/src/vector.rs
/// 长期向量记忆：Qdrant 封装
pub struct VectorMemory {
client:     qdrant_client::Qdrant,
collection: String,
embedder:   Arc<dyn Embedder>,
}

impl VectorMemory {
/// 语义搜索：找到最相关的历史经验
pub async fn search(&self, query: &str, k: usize)
-> Result<Vec<MemoryChunk>>
{
let vec = self.embedder.embed(query).await?;
let results = self.client
.search_points(qdrant_client::qdrant::SearchPoints {
collection_name: self.collection.clone(),
vector: vec,
limit: k as u64,
with_payload: Some(true.into()),
..Default::default()
}).await?;
// 解析并返回 MemoryChunk 列表
Ok(results.result.into_iter()
.filter_map(│p│ MemoryChunk::from_point(p).ok())
.collect())
}

pub async fn upsert(&self, chunk: MemoryChunk) -> Result<()> {
let vec = self.embedder.embed(&chunk.content).await?;
// 写入 Qdrant...
Ok(())
}
}
```
| **5.4  工具注册表 (tool_registry) — ~310 行** |
| --- |

工具注册表使用 trait 对象实现动态工具注册，支持在运行时热插拔新工具。每个工具实现 Tool trait，注册后即可被 Planner 引用。

```rust
// crates/tools/src/lib.rs

#[async_trait]
pub trait Tool: Send + Sync {
fn name(&self) -> &str;
fn description(&self) -> &str;
fn schema(&self) -> serde_json::Value;  // JSON Schema 格式参数定义
async fn call(&self, args: serde_json::Value) -> Result<ToolOutput>;
}

pub struct ToolRegistry {
tools: DashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
pub fn register(&self, tool: Arc<dyn Tool>) {
self.tools.insert(tool.name().to_string(), tool);
}

pub async fn call(&self, name: &str, args: serde_json::Value)
-> Result<ToolOutput>
{
let tool = self.tools.get(name)
.ok_or_else(││ anyhow::anyhow!("tool not found: {name}"))?;
tool.call(args).await
}

/// 生成所有工具的描述（注入 LLM prompt）
pub fn describe_all(&self) -> Vec<serde_json::Value> {
self.tools.iter()
.map(│e│ serde_json::json!({
"name": e.name(),
"description": e.description(),
"parameters": e.schema()
}))
.collect()
}
}

// 示例内置工具
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
fn name(&self) -> &str { "bash" }
fn description(&self) -> &str { "Run a bash command and return stdout/stderr" }
fn schema(&self) -> serde_json::Value {
serde_json::json!({
"type": "object",
"properties": { "command": {"type":"string"} },
"required": ["command"]
})
}
async fn call(&self, args: serde_json::Value) -> Result<ToolOutput> {
let cmd = args["command"].as_str().unwrap_or("");
let out = tokio::process::Command::new("bash")
.arg("-c").arg(cmd)
.output().await?;
Ok(ToolOutput::text(String::from_utf8_lossy(&out.stdout).into()))
}
}
```
| **5.5  反思评估器 (reflector) — ~290 行** |
| --- |

Reflector 对执行结果打分并归因，生成 Insight 结构，是连接执行与进化的关键桥梁。

```rust
// crates/reflector/src/reflector.rs

pub struct Reflector {
llm:    Arc<dyn LlmAdapter>,
scorer: Scorer,
}

impl Reflector {
pub async fn reflect(&self, result: &ExecutionResult) -> Result<Insight> {
// 1. 结构化评分（成功率、延迟、输出质量）
let score = self.scorer.score(result);

// 2. 调用 LLM 进行语义归因（只在失败时调用，节省 token）
let lesson = if score < 0.0 {
self.llm.complete(self.build_attribution_prompt(result)).await?
} else {
format!("Strategy succeeded with score {:.2}", score)
};

// 3. 将 lesson 向量化（用于长期记忆存储）
let embedding = self.llm.embed(&lesson).await?;

Ok(Insight {
strategy_id: result.strategy_id(),
score,
embedding,
lesson,
})
}
}

/// 评分器：纯函数，无 IO
pub struct Scorer {
success_weight:  f64,  // 默认 0.6
latency_weight:  f64,  // 默认 0.2
quality_weight:  f64,  // 默认 0.2
latency_target:  u64,  // 目标延迟（ms），默认 2000
}

impl Scorer {
pub fn score(&self, result: &ExecutionResult) -> f64 {
let success  = if result.success { 1.0 } else { -1.0 };
let latency  = 1.0 - (result.duration_ms as f64
/ self.latency_target as f64).min(1.0);
let quality  = self.measure_quality(result);
self.success_weight  * success
+ self.latency_weight  * latency
+ self.quality_weight  * quality
}
}
```
| **5.6  大模型适配层 (llm_adapter) — ~380 行** |
| --- |

LLM 适配层将不同模型的 API 差异屏蔽在 trait 后面。新增模型只需实现 LlmAdapter trait，无需修改其他代码。

```rust
// crates/llm/src/adapter.rs

#[async_trait]
pub trait LlmAdapter: Send + Sync {
async fn complete(&self, prompt: String) -> Result<String>;
async fn complete_stream(&self, prompt: String)
-> Result<impl futures::Stream<Item = Result<String>>>;
async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

// crates/llm/src/anthropic.rs
pub struct AnthropicAdapter {
client:     reqwest::Client,
api_key:    String,
model:      String,  // "claude-sonnet-4-5-20251001"
max_tokens: u32,
}

#[async_trait]
impl LlmAdapter for AnthropicAdapter {
async fn complete(&self, prompt: String) -> Result<String> {
let resp = self.client
.post("https://api.anthropic.com/v1/messages")
.header("x-api-key", &self.api_key)
.header("anthropic-version", "2023-06-01")
.json(&serde_json::json!({
"model": self.model,
"max_tokens": self.max_tokens,
"messages": [{"role":"user","content": prompt}]
}))
.send().await?.json::<serde_json::Value>().await?;

Ok(resp["content"][0]["text"]
.as_str().unwrap_or("").to_string())
}

async fn embed(&self, text: &str) -> Result<Vec<f32>> {
// 使用 voyage-3 或 text-embedding-3-small
todo!()
}

async fn complete_stream(&self, prompt: String)
-> Result<impl futures::Stream<Item = Result<String>>> {
// 流式实现，返回 SSE 解析后的 token stream
todo!()
}
}
```
| **5.7  异步调度器 (scheduler) — ~260 行** |
| --- |

Scheduler 接收 Plan（DAG 结构），按依赖拓扑顺序并发执行各步骤，使用 Semaphore 控制最大并发数。

```rust
// crates/scheduler/src/scheduler.rs
use tokio::sync::Semaphore;
use futures::future::join_all;

pub struct Scheduler {
registry:    Arc<ToolRegistry>,
semaphore:   Arc<Semaphore>,   // 最大并发工具调用数
}

impl Scheduler {
pub async fn execute(&self, plan: Plan) -> Result<ExecutionResult> {
let mut completed: HashMap<uuid::Uuid, StepOutput> = HashMap::new();
let start = std::time::Instant::now();

// 拓扑排序后按层并发执行
for layer in plan.dag.topological_layers() {
let futs: Vec<_> = layer.iter().map(│step│ {
let reg  = Arc::clone(&self.registry);
let sem  = Arc::clone(&self.semaphore);
let step = step.clone();
async move {
let _permit = sem.acquire().await?;
let out = reg.call(&step.tool, step.args.clone()).await;
anyhow::Ok((step.id, out?))
}
}).collect();

for (id, out) in join_all(futs).await
.into_iter().filter_map(│r│ r.ok())
{
completed.insert(id, out);
}
}

let success = completed.len() == plan.steps.len();
Ok(ExecutionResult {
plan_id:     plan.id,
outputs:     completed.into_values().collect(),
success,
duration_ms: start.elapsed().as_millis() as u64,
})
}
}
```

---

# 6  程序入口 (main.rs)
```rust
// src/main.rs  (~80 行)
use clap::Parser;

#[derive(Parser)]
#[command(name = "hermes", about = "Small Hermes Agent")]
struct Cli {
#[arg(short, long, default_value = "config/default.toml")]
config: String,
#[arg(short, long)]
task: Option<String>,       // 单次任务模式
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
// 初始化日志
tracing_subscriber::fmt()
.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
.init();

let cli  = Cli::parse();
let cfg  = Config::from_file(&cli.config)?;

// 组装 Agent 所需的所有依赖
let memory   = Arc::new(VectorMemory::new(&cfg.qdrant).await?);
let llm      = Arc::new(AnthropicAdapter::new(&cfg.llm));
let tools    = Arc::new(ToolRegistry::default());
tools.register(Arc::new(BashTool));
tools.register(Arc::new(WebSearchTool::new(&cfg.search)));

let evolution = Arc::new(EvolutionEngine::new(cfg.learning_rate, Arc::clone(&memory)));
let planner   = Planner::new(Arc::clone(&llm), Arc::clone(&evolution));
let scheduler = Scheduler::new(Arc::clone(&tools), cfg.max_concurrency);
let reflector = Reflector::new(Arc::clone(&llm));

let mut agent = SmallHermesAgent {
planner, scheduler, reflector, evolution,
working_memory: WorkingMemory::new(cfg.working_memory_size),
};

// 构造 Context（支持 Ctrl+C 优雅退出）
let ctx = Context::new_with_ctrlc(cli.task);
agent.run_loop(ctx).await
}
```

---

# 7  推荐实现顺序
建议按以下顺序实现，每一步都可以独立测试，避免集成困难。

## Phase 1 — 骨架（预计 2-3 天）
- 创建 workspace，配置 Cargo.toml，安装所有依赖

- 定义所有核心数据结构（Observation、Plan、Step、ExecutionResult、Insight）

- 定义 HermesAgent trait 和 run_loop 默认实现

- 实现最简 Context（包含 should_stop 标志位）

- 编写 main.rs 框架，确保能编译通过（各模块用 todo!() 占位）

## Phase 2 — 工具层（预计 1-2 天）
- 实现 Tool trait 和 ToolRegistry

- 实现 BashTool、WebSearchTool 两个内置工具

- 实现 Scheduler 的拓扑排序并发执行逻辑

- 编写工具调用的集成测试

## Phase 3 — LLM 层（预计 1 天）
- 实现 LlmAdapter trait

- 实现 AnthropicAdapter（先做非流式版本）

- 实现 Embedder（用于向量化）

## Phase 4 — 规划与记忆（预计 2 天）
- 实现 WorkingMemory（Ring buffer）

- 搭建 Qdrant 本地实例，实现 VectorMemory

- 实现 Planner（设计 prompt，解析 JSON 步骤）

## Phase 5 — 进化闭环（预计 2 天）
- 实现 Scorer 评分器

- 实现 Reflector 反思器（含 LLM 归因调用）

- 实现 EvolutionEngine（权重更新 + 记忆写入）

- 端到端集成测试：完整跑通一次五步循环

## Phase 6 — 打磨（持续）
- 添加 tracing 日志覆盖关键路径

- 实现自适应学习率衰减

- 增加配置文件支持（TOML）

- 编写压力测试，验证并发安全


---

# 8  快速启动
## 8.1  环境准备
- 安装 Rust：curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh

- 安装 Docker（运行 Qdrant）：https://docs.docker.com/get-docker/

- 获取 Anthropic API Key：https://console.anthropic.com

## 8.2  启动 Qdrant
```bash
# 启动本地 Qdrant 向量数据库
docker run -d --name qdrant \
-p 6333:6333 -p 6334:6334 \
-v $(pwd)/qdrant_data:/qdrant/storage \
qdrant/qdrant
```
## 8.3  配置环境变量
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export HERMES_LOG="info"
```
## 8.4  运行 Agent
```bash
# 构建（首次较慢，约 1-2 分钟）
cargo build --release

# 单次任务模式
cargo run --release -- --task "帮我查找最新的 Rust 异步编程最佳实践"

# 持续循环模式
HERMES_LOG=debug cargo run --release

# 运行测试
cargo test
cargo test --workspace
```

---

# 9  关键技术决策说明
| **决策点** | **选择** | **原因** |
| --- | --- | --- |
| 并发模型 | Tokio async/await | 工具调用 IO 密集，异步比线程池节省资源 10x |
| 权重存储 | DashMap（无锁） | 避免 RwLock 竞争，读多写少场景性能最优 |
| 长期记忆 | Qdrant 向量库 | 支持语义相似度检索，比 BM25 召回质量高 |
| 学习率更新 | AtomicU64 存 f64 bits | 无需 Mutex，CAS 操作原子更新浮点数 |
| 工具注册 | Arc<dyn Tool> | trait 对象支持运行时动态注册，无需重编译 |
| 错误处理 | anyhow + thiserror | 快速原型用 anyhow，库边界用 thiserror |
| 配置格式 | TOML | 人类可读，serde 原生支持，Rust 生态主流 |
| DAG 执行 | 拓扑排序分层 | 最大化并发度，同层步骤全部并发执行 |

*⚠ DashMap 在极高竞争（**>**32 线程同时写同一 key）时性能会下降。如果 Agent 需要超高并发，考虑换用 tokio::sync::Mutex**<**HashMap**>** + 分片。*

*💡 Qdrant 的嵌入维度需要与 Embedder 输出维度匹配。Anthropic voyage-3 输出 1024 维，OpenAI text-embedding-3-small 输出 1536 维，创建 collection 时需正确指定。*


---

# 10  测试策略
## 10.1  单元测试
- Scorer：纯函数，覆盖成功/失败/超时三种场景

- DependencyGraph：测试环路检测、拓扑排序正确性

- WorkingMemory：测试容量溢出时的淘汰行为

- EvolutionEngine：mock 掉 VectorMemory，验证权重更新逻辑

## 10.2  集成测试
- 工具调用：使用 mockito 或 wiremock mock HTTP 服务

- LLM 调用：录制真实 API 响应存为 fixture，回放测试

- 完整循环：用固定 seed 的 mock LLM 跑通五步循环

## 10.3  并发安全测试
```rust
// 使用 loom 进行并发模型检验（可选，但推荐）
#[cfg(test)]
mod concurrent_tests {
#[test]
fn evolution_engine_concurrent_update() {
// 启动 100 个并发 update，验证最终权重一致性
let rt = tokio::runtime::Runtime::new().unwrap();
rt.block_on(async {
let engine = Arc::new(EvolutionEngine::new(0.01, mock_memory()));
let futs: Vec<_> = (0..100).map(│i│ {
let e = Arc::clone(&engine);
async move {
e.update(Insight {
strategy_id: "test".into(),
score: if i % 2 == 0 { 1.0 } else { -1.0 },
..Default::default()
}).await
}
}).collect();
futures::future::join_all(futs).await;
// 验证 strategy_weights["test"] 在合理范围内
});
}
}
```
*── 文档结束 ──*

Small Hermes Agent · 详细实现架构文档 v1.0