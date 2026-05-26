# 用 Rust 从零构建自进化 AI Agent：Hermes 架构全解析

> 一个具备自我进化能力的 AI 智能体，2000+ 行 Rust 代码，13 项生产级特性，从 CLI 到 MCP 协议全覆盖。

---

## 一、什么是 Hermes？

Hermes（赫耳墨斯，希腊神话中的信使之神）是一个用 Rust 编写的 **自进化 AI Agent 框架**。它的核心思想很简单却强大：**智能体不仅执行任务，还会从每次执行中学习，持续优化自己的策略**。

与普通的 LLM 聊天机器人不同，Hermes 运行一个固定的五步循环：

```
┌─────────────────────────────────────────────────────────┐
│                    Hermes Agent Loop                      │
│                                                           │
│   ┌──────────┐    ┌──────────┐    ┌──────────┐           │
│   │ OBSERVE  │───▶│   PLAN   │───▶│ EXECUTE  │           │
│   │ 观察环境  │    │ 任务分解  │    │ DAG 执行  │           │
│   └──────────┘    └──────────┘    └──────────┘           │
│         ▲                                │                │
│         │                                ▼                │
│   ┌──────────┐    ┌────────────────────────────────┐     │
│   │  EVOLVE  │◀───│            REFLECT              │     │
│   │ 进化学习  │    │      评分 + 错误归因 + 嵌入      │     │
│   └──────────┘    └────────────────────────────────┘     │
│         │                                                │
│         ▼                                                │
│   策略权重更新 (DashMap 无锁并发)                           │
│   长期记忆写入 (向量数据库 Qdrant)                          │
└─────────────────────────────────────────────────────────┘
```

每次循环结束后，智能体会生成一个中文摘要。多次对话的上下文会被保留，超过阈值时自动压缩。更重要的是，**每次执行的成功/失败都会反馈到进化引擎**，调整策略权重，让智能体越来越"聪明"。

---

## 二、项目架构：12 个 Crate 的精密协作

Hermes 采用 Rust 工作空间（workspace）组织，包含 **13 个子 crate** 和 1 个顶层二进制入口：

```
hermes/                           ← CLI 入口 (src/main.rs)
crates/
├── agent-core/                   ← 核心 trait、数据类型、DAG
├── hermess-agent/                ← SmallHermesAgent 具体实现
├── planner/                      ← LLM 驱动任务分解为 DAG
├── scheduler/                    ← 拓扑层并发执行 + 重试 + cron
├── evolution/                    ← 无锁策略权重进化引擎
├── reflector/                    ← 结果评分 + 错误归因
├── memory/                       ← 工作记忆 + Qdrant 向量记忆
├── tools/                        ← 工具注册表 + 内置工具 + 插件系统
├── llm/                          ← LLM 适配器 (Anthropic/OpenAI)
├── tui/                          ← Ratatui 终端用户界面
├── hermess-web/                  ← HTTP API + 企业微信回调
└── mcp/                          ← MCP 协议 stdio 服务器
```

**数据流全景图：**

```
                         ┌─────────────────┐
                         │   User Input     │
                         └────────┬────────┘
                                  │
                    ┌─────────────▼──────────────┐
                    │     Observation             │
                    │  user_input + memory_ctx    │
                    └─────────────┬──────────────┘
                                  │
              ┌───────────────────▼────────────────────┐
              │           Planner.plan()                │
              │  ┌──────────────────────────────┐      │
              │  │ LLM Prompt: 工具 + 记忆 + 约束 │      │
              │  │ Evolution.best_strategy()     │      │
              │  └──────────────────────────────┘      │
              └───────────────────┬────────────────────┘
                                  │
                    ┌─────────────▼──────────────┐
                    │    Plan { steps[], dag }    │
                    └─────────────┬──────────────┘
                                  │
              ┌───────────────────▼────────────────────┐
              │         Scheduler.execute()             │
              │  ┌──────────────────────────────┐      │
              │  │ 拓扑分层 → 同层并发执行          │      │
              │  │ {{step_N.output}} 模板解析      │      │
              │  │ 失败重试 + 备选工具回退          │      │
              │  └──────────────────────────────┘      │
              └───────────────────┬────────────────────┘
                                  │
                    ┌─────────────▼──────────────┐
                    │   ExecutionResult { ... }   │
                    └─────────────┬──────────────┘
                                  │
              ┌───────────────────▼────────────────────┐
              │         Reflector.reflect()              │
              │  ┌──────────────────────────────┐      │
              │  │ Scorer: 结构化评分 [-1, +1]    │      │
              │  │ LLM 错误归因 → lesson 教训     │      │
              │  │ 向量嵌入 → 长期记忆            │      │
              │  └──────────────────────────────┘      │
              └───────────────────┬────────────────────┘
                                  │
                    ┌─────────────▼──────────────┐
                    │     Insight { score,        │
                    │       strategy_id, lesson } │
                    └─────────────┬──────────────┘
                                  │
              ┌───────────────────▼────────────────────┐
              │       EvolutionEngine.update()          │
              │  ┌──────────────────────────────┐      │
              │  │ 自适应学习率: lr/√(n+1)        │      │
              │  │ 权重 = clamp(w + score*lr)     │      │
              │  │ 异步写入 Qdrant 长期记忆        │      │
              │  └──────────────────────────────┘      │
              └────────────────────────────────────────┘
                                  │
                                  ▼
                         下一轮循环 (turn++)
```

---

## 三、核心组件深度解析

### 3.1 Planner：LLM 驱动的任务分解器

Planner 是整个智能体的"大脑"，负责将用户的自然语言任务分解为机器可执行的步骤 DAG。

**设计要点：**

- **Prompt 工程**：构建包含可用工具列表、schema 定义、数据传递规则（`{{step_N.output}}`）的详细提示词
- **工具选择指南**：明确告诉 LLM，"对话类任务直接用 reply，不要动用 bash"
- **双重解析保护**：如果首次 JSON 解析失败，使用更严格的提示重试一次
- **策略选择**：为每个步骤从进化引擎查询最佳策略权重

```rust
// 规划器返回的 DAG 结构
pub struct Plan {
    pub id: Uuid,
    pub steps: Vec<Step>,         // 执行步骤列表
    pub dag: DependencyGraph,     // 步骤间依赖关系
}

pub struct Step {
    pub tool: String,             // 工具名称 (bash/reply/web_search...)
    pub args: serde_json::Value,  // 工具参数
    pub depends: Vec<Uuid>,       // 依赖步骤 ID
    pub strategy: String,         // 选中的策略名
    pub tool_candidates: Vec<String>, // 失败时的备选工具
    pub delegable: bool,          // 是否可委托子 Agent
}
```

### 3.2 Scheduler：DAG 并发执行引擎

Scheduler 将 Plan 中的步骤按拓扑排序后分层执行。**同一层的步骤可以并发运行**，大大提升效率。

```
步骤依赖图:                  拓扑分层执行:
                                  
  [step0] ──→ [step2]        Layer 0:  [step0] ─── 并发 ───┐
     │           │                                  │        │
     └──→ [step1] ┘           Layer 1:  [step1] ─── 并发 ───┤
                                                          │
                               Layer 2:         [step2] ◀──┘
```

**执行细节：**

1. **并发控制**：通过 `tokio::sync::Semaphore` 限制最大并发数（默认 10）
2. **模板解析**：`{{step_N.output}}` 在参数中动态替换为前置步骤的实际输出
3. **重试机制**：失败后最多重试 N 次（默认 3），每次注入 `_previous_error` 上下文
4. **备选工具回退**：所有重试失败后，自动尝试 `tool_candidates` 列表中的替代工具
5. **事件发射**：每个步骤开始/完成时通过 channel 发射事件，TUI 实时更新

### 3.3 Evolution Engine：自进化的核心

这是 Hermes 最独特的设计——**一个无锁、自适应学习率的策略权重进化系统**。

```
进化引擎架构:

   Insight { strategy_id, score, lesson }
                │
                ▼
   ┌──────────────────────────────┐
   │  1. 统计更新 (RwLock)         │
   │     positive/negative/avg     │
   │  2. 自适应学习率衰减           │
   │     lr_t = lr_0 / √(n+1)     │
   │  3. 原子权重更新 (DashMap)     │
   │     w += score × lr_t        │
   │  4. 异步记忆写入 (tokio)      │
   │     失败不阻塞主循环           │
   └──────────────────────────────┘
```

**关键技术选型：**

| 组件 | 技术 | 原因 |
|------|------|------|
| 策略权重 | `DashMap<K,V>` | 无锁并发 HashMap，原子读-修改-写 |
| 学习率 | `AtomicU64` (存 f64 bits) | 避免浮点原子操作的平台差异 |
| 统计信息 | `parking_lot::RwLock` | 读写分离，比 std::Mutex 更快 |
| 长期记忆 | Qdrant 向量数据库 | 语义搜索，支持余弦相似度 |

**学习率衰减**确保初期探索激进（大步伐学习），后期收敛稳定（小步伐微调）：

```
lr(n) = lr₀ / √(n + 1)

n=0:    lr = 0.100    ← 初期快速学习
n=10:   lr = 0.030
n=100:  lr = 0.010
n=1000: lr = 0.003    ← 后期精细调整
```

### 3.4 Reflector：结构化反馈系统

Reflector 负责对执行结果进行多维评分，并在失败时进行智能归因。

```rust
// 评分公式 (范围 [-1.0, +1.0])
score = success_weight × (±1.0)              // 成功/失败权重 0.6
      + latency_weight × (1 - min(t/target, 1))  // 延迟权重 0.2
      + quality_weight × (successful/total)   // 质量权重 0.2
```

- **成功时**：生成简洁的正面反馈
- **失败时**：调用 LLM 进行**错误归因**，提取可学习的教训（lesson）
- **向量化**：将教训通过嵌入模型转为向量，存入长期记忆供未来检索

### 3.5 Memory 系统：短期 + 长期双层记忆

```
┌─────────────────────────────────────────────┐
│              记忆系统架构                      │
│                                               │
│  短期记忆 (Working Memory)                     │
│  ┌──────────────────────────────────────┐    │
│  │  固定容量环形缓冲区 (默认 100 条)       │    │
│  │  recent(5) → 注入 Observation         │    │
│  │  每次 execute 后追加                   │    │
│  └──────────────────────────────────────┘    │
│                                               │
│  长期记忆 (Vector Memory)                      │
│  ┌──────────────────────────────────────┐    │
│  │  Qdrant 向量数据库                    │    │
│  │  语义搜索 → 检索相关历史经验           │    │
│  │  降级方案: 内存余弦相似度              │    │
│  └──────────────────────────────────────┘    │
│                                               │
│  知识库预加载 (Knowledge Base)                 │
│  ┌──────────────────────────────────────┐    │
│  │  目录扫描 → 文本分块 → 嵌入 → upsert  │    │
│  │  支持 30+ 种文件扩展名                │    │
│  │  二值检测 + 大文件跳过                 │    │
│  └──────────────────────────────────────┘    │
└─────────────────────────────────────────────┘
```

### 3.6 Tool 系统：插件化可扩展

内置 5 个工具，通过 **TOML manifest** 可以无限扩展：

```
内置工具:
  reply       → 自然语言回复（对话任务的默认工具）
  bash        → Shell 命令执行（受 DangerGuard 保护）
  read_file   → 文件读取（>10KB 自动截断）
  write_file  → 文件写入（自动创建父目录）
  web_search  → Brave Search API 搜索

插件系统:
  plugins/
  └── hello-world/
      └── plugin.toml   ← TOML 定义工具名、描述、schema、命令模板

  支持两种类型:
  Shell 插件:  command = "echo 'Hello, $ARG.name!'"
  Script 插件: interpreter + script 路径，JSON args 通过 stdin 传入
```

**DangerGuard 危险命令守卫**内置 36 种危险模式匹配：

```
rm -rf /, sudo, chmod 777, git push --force,
mkfs.*, dd if=, fork bomb (:(){ :|:& };:),
/etc/passwd, /dev/sda, curl | sh ...
```

三种策略：`Ask`（弹窗确认）、`Skip`（放行）、`Deny`（自动拒绝）。

---

## 四、13 项生产级特性一览

| 阶段 | 特性 | 说明 |
|------|------|------|
| **Phase 1** | F1 危险命令确认 | 36 种危险模式，Ask/Skip/Deny 策略 |
| | F2 会话保存/恢复 | JSON 序列化，`--save`/`--resume` |
| | F3 配置预设 Profile | `--profile dev/prod` |
| | F4 Token 用量追踪 | 多模型定价表，费用估算 |
| **Phase 2** | F5 失败重试+重规划 | 最多 3 次重试 + 备选工具 + LLM 重新规划 |
| | F6 TUI 流式输出 | Plan/Summary 阶段实时 token 显示 |
| | F7 上下文自动压缩 | 超过阈值→LLM 压缩旧对话为摘要 |
| | F8 插件系统 | TOML manifest，Shell/Script 两种插件 |
| **Phase 3** | F9 HTTP API | `--serve` 启动 axum 服务器 |
| | F10 知识库预加载 | `--knowledge-base` RAG 支持 |
| | F11 多 Agent 并行 | `delegable` 标记 + SubAgent 事件 |
| | F12 定时任务 | `--schedule "0 */6 * * *"` cron 表达式 |
| **Phase 4** | F13 MCP 协议 | JSON-RPC 2.0 stdio server |

---

## 五、TUI 终端界面

Hermes 配备了一个基于 **Ratatui + Crossterm** 的全功能终端 UI：

```
┌─ Hermes Agent ── 第 3 轮 ── ◉ Executing ───────────────────────┐
│                                                                   │
│  ┌─ PLAN ──┬── EXEC ─────────────────────────────┐ ┌─ Evolution ─┐
│  │          │                                      │ │             │
│  │ LLM 流式 │  ✓ bash (0.3s)                      │ │ Stats       │
│  │ 输出...  │     echo "Hello World"              │ │ 胜率: 75%   │
│  │          │  ✗ web_search (2.1s)                │ │             │
│  │          │     timeout                         │ │ Weights     │
│  │          │  ◎ reply (1.2s)                     │ │ fast  ██▌   │
│  │          │     正在生成...                      │ │ safe  ████▍ │
│  │          │                                      │ │             │
│  └──────────┴────────────────────────────────────┘ │             │
│                                                                   │
│  ┌─ Mini Log ──────────────────────────────────────────────────┐ │
│  │ ✓ abc123 (0.3s): echo "Hello World"                         │ │
│  │ ✗ def456 (2.1s): timeout                                      │ │
│  └──────────────────────────────────────────────────────────────┘ │
│                                                                   │
│  TAB:面板  j/k:滚动  Enter:详情  q:退出    帮助: h                │
└───────────────────────────────────────────────────────────────────┘
```

**设计理念**：极简、深色主题、无闪烁动画、2-3 个核心快捷键提示。每个阶段自动调整面板分割比例（规划阶段左侧 85%，执行阶段 80%）。

---

## 六、LLM 适配器：Provider 无关设计

通过 `LlmAdapter` trait 实现统一的 LLM 接口：

```rust
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    async fn complete(&self, prompt: String) -> Result<String>;
    async fn complete_stream(&self, prompt: String)
        -> Result<Box<dyn Stream<Item = Result<String>>>>;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn last_usage(&self) -> Option<TokenUsage>;
}
```

**两个适配器实现：**

| 适配器 | API 端点 | 流式格式 | 嵌入 |
|--------|---------|----------|------|
| `OpenAIAdapter` | `{base_url}/chat/completions` | SSE `choices[0].delta.content` | `text-embedding-3-small` |
| `AnthropicAdapter` | `api.anthropic.com/v1/messages` | SSE `delta.text` | 零向量（不支持） |

`OpenAIAdapter` 兼容所有 OpenAI API 兼容服务（DeepSeek、Groq 等），只需配置 `base_url`。

流式解析器 `SseChunkStream` 同时处理两种格式的差异，对外暴露统一的 `Stream<Item = Result<String>>` 接口。

---

## 七、HTTP API 与 MCP 协议

### HTTP Server 模式 (`--serve 8080`)

```
GET  /health       → "ok"
POST /agent/run    → {"task": "帮我查天气"} → {"summary":"...", "turn":1, "success":true}
```

Agent 包裹在 `Arc<Mutex<SmallHermesAgent>>` 中，请求在独立的 tokio 任务中执行。

### MCP Server 模式 (`--mcp-server`)

实现了完整的 **Model Context Protocol** stdio 服务器：

```
stdin  → JSON-RPC 2.0 Request
stdout → JSON-RPC 2.0 Response
stderr → 日志输出

支持的方法:
  initialize            → 返回协议版本 + 能力声明
  tools/list            → 返回所有已注册工具的定义
  tools/call            → 调用指定工具并返回结果
```

可以被任何支持 MCP 的客户端（如 Claude Desktop、VS Code 插件）直接调用。

---

## 八、Rust 工程实践亮点

### 8.1 并发模型

```rust
// 无锁策略权重存储
strategy_weights: Arc<DashMap<String, f64>>

// f64 的原子操作
learning_rate_bits: AtomicU64  // 存 f64::to_bits()
insight_count: AtomicU64       // 自适应学习率衰减

// 读写分离统计
stats: parking_lot::RwLock<InsightStats>

// 工具调用并发控制
concurrency: ConcurrencyLimit  // tokio::sync::Semaphore
```

### 8.2 错误处理与降级

- **Qdrant 不可达**：自动降级为内存余弦相似度搜索
- **嵌入 API 失败**：返回零向量（标记为降级）
- **LLM 总结失败**：回退到原始执行输出的格式化摘要
- **压缩失败**：静默保留原始历史，不影响主流程
- **插件目录不存在**：跳过，不影响启动

### 8.3 事件驱动架构

所有组件通过 `tokio::mpsc::UnboundedSender<AgentEvent>` 发射事件，TUI、HTTP、MCP 三种观察者模式可以同时订阅：

```
Planner ──→ PlanStreamingToken ──→ TUI 实时显示
Scheduler ──→ StepStarted/Completed ──→ TUI 进度更新
Agent ──→ SummaryStreamingToken ──→ TUI 摘要流式输出
Agent ──→ ReplanNeeded ──→ TUI 状态切换
```

---

## 九、快速开始

```bash
# 默认配置运行（DeepSeek API）
cargo run -- -t "帮我写一个 Python 的快速排序"

# TUI 交互模式
cargo run -- --tui

# 使用 Anthropic Claude
cargo run -- --provider anthropic --model claude-sonnet-4-5-20251001 -t "..."

# HTTP 服务器模式
cargo run -- --serve 8080

# MCP 服务器模式
cargo run -- --mcp-server

# 定时任务（每 6 小时）
cargo run -- --schedule "0 */6 * * *" -t "检查系统状态"

# 预加载知识库
cargo run -- --knowledge-base ./docs --knowledge-base ./src -t "解释项目架构"

# 会话保存与恢复
cargo run -- --save /tmp/session.json -t "开始一个任务"
cargo run -- --resume /tmp/session.json -t "继续上次的任务"
```

---

## 十、总结

Hermes 展示了如何用 **纯 Rust** 构建一个生产级的自进化 AI Agent 框架。它的核心价值在于：

1. **五步自进化循环**：观察→规划→执行→反思→进化，每次执行都在学习和优化
2. **无锁并发设计**：DashMap + AtomicU64 + RwLock，多组件并行无竞争
3. **插件化工具系统**：TOML 定义即可扩展，Shell/Script 两种模式
4. **完整的工程实践**：错误降级、流式输出、会话持久化、上下文压缩
5. **多协议支持**：CLI / TUI / HTTP / MCP 四种入口，覆盖不同使用场景

项目仓库：[github.com/nova/hermess](https://github.com/nova/hermess)

---

*本文由 Claude Code 辅助撰写，所有架构图均为原创绘制。*
