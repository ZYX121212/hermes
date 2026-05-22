# Hermes Agent

一个极简自进化 AI Agent，用 Rust 实现。遵循五步反馈循环，每次执行后自动学习和优化策略权重。

```
Observe → Plan → Execute → Reflect → Evolve
   ↑________________________________________|
```

## 架构

```
hermes/
├── crates/
│   ├── agent-core/     # 核心 trait、数据结构、主循环
│   ├── evolution/      # 无锁策略权重进化引擎
│   ├── planner/        # LLM 驱动的任务分解
│   ├── memory/         # 短期工作记忆 + 长期向量记忆
│   ├── tools/          # 动态工具注册 + 内置工具
│   ├── reflector/      # 结果评分和错误归因
│   ├── llm/            # LLM 适配层（Anthropic / OpenAI / DeepSeek）
│   └── scheduler/      # DAG 拓扑排序并发执行
├── config/             # TOML 配置文件
├── src/main.rs         # CLI 入口
└── tests/              # 集成测试
```

## 快速开始

### 前置条件

- Rust (Edition 2021)
- 至少一个 LLM 提供商的 API Key

### 安装 & 构建

```bash
git clone <repo-url> && cd hermes
cargo build --release
```

### 支持的 LLM 提供商

| 提供商 | 环境变量 | 配置值 |
|---|---|---|
| Anthropic | `ANTHROPIC_API_KEY` | `provider = "anthropic"` |
| OpenAI | `OPENAI_API_KEY` | `provider = "openai"` |
| DeepSeek | `DEEPSEEK_API_KEY` | `provider = "deepseek"` |
| 任意 OpenAI 兼容 API | — | `provider = "openai"` + `base_url` |

### 运行

```bash
# 单次任务模式 — 执行一次后自动退出
cargo run --release -- --config config/deepseek.toml --task "你的任务描述"

# DeepSeek 示例
export DEEPSEEK_API_KEY="sk-..."
cargo run --release -- --config config/deepseek.toml --task "查找最新的 Rust 异步编程最佳实践"

# Anthropic 示例
export ANTHROPIC_API_KEY="sk-ant-..."
cargo run --release -- --task "帮我分析这个项目的代码结构"

# OpenAI 示例
export OPENAI_API_KEY="sk-..."
cargo run --release -- --config config/default.toml --task "写一个 Python 快速排序"

# 持续循环模式 — Ctrl+C 退出
RUST_LOG=info cargo run --release

# Debug 模式 — 查看完整执行细节
RUST_LOG=debug cargo run --release -- --task "echo hello"
```

## 配置

### 默认配置 (`config/default.toml`)

```toml
learning_rate = 0.1          # 学习率
working_memory_size = 100    # 短期记忆容量
max_concurrency = 10         # 最大并发工具调用数

[llm]
provider = "anthropic"
model = "claude-sonnet-4-5-20251001"
max_tokens = 4096
# api_key = "sk-..."         # 可写死或使用环境变量

[qdrant]
url = "http://localhost:6334"
collection = "hermes_memory"
embedding_dim = 1024

[search]
# Brave Search API（可选）
# api_key = "BSA-..."
endpoint = "https://api.search.brave.com/res/v1/web/search"

[scorer]
success_weight = 0.6         # 成功率权重
latency_weight = 0.2         # 延迟权重
quality_weight = 0.2         # 输出质量权重
latency_target_ms = 2000     # 目标延迟（毫秒）
```

### API Key 优先级

1. 配置文件中的 `api_key` 字段
2. 对应环境变量（`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `DEEPSEEK_API_KEY`）

### DeepSeek 专用配置 (`config/deepseek.toml`)

DeepSeek 使用 OpenAI 兼容 API，自动配置如下：
- 端点：`https://api.deepseek.com/v1`
- 模型：`deepseek-chat`
- 嵌入：DeepSeek 不支持，自动降级为零向量

```toml
[llm]
provider = "deepseek"
model = "deepseek-chat"
max_tokens = 4096
```

### 生产环境配置 (`config/production.toml`)

采用更保守的参数：
- 学习率 0.05
- 内存容量 500
- 最大并发 50
- 评分权重偏重成功率（0.7）

## 内置工具

| 工具 | 用途 |
|---|---|
| `bash` | 执行 shell 命令 |
| `web_search` | Brave Search API 网页搜索（可选配置） |

## CLI 选项

```
Usage: hermes [OPTIONS]

Options:
  -c, --config <CONFIG>  配置文件路径 [default: config/default.toml]
  -t, --task <TASK>      单次任务（执行一次后自动退出）
  -h, --help             打印帮助信息
```

## 运行示例

```bash
# 写代码
$ hermes --config config/deepseek.toml --task "写一个 Rust 冒泡排序"
# → 写入 bubble_sort.rs → 编译 → 运行 → 输出: [11, 12, 22, 25, 34, 64, 90]

# 执行计算
$ hermes --config config/deepseek.toml --task "计算 1+2+3 的和"
# → bash: echo $((1+2+3)) → 结果: 6 → 得分: 0.9996

# 信息检索（需配置 web_search）
$ hermes --task "搜索 Rust 1.85 的新特性"
# → web_search → 整理结果 → 输出摘要
```

## 进化机制

每次执行后，Agent 自动学习：

1. **评分**：根据成功率、延迟、输出质量计算得分 [-1, 1]
2. **归因**：失败时调用 LLM 分析原因
3. **更新权重**：`weight += score × learning_rate`
4. **衰减**：学习率随时间递减 `lr_t = lr_0 / sqrt(n + 1)`
5. **记忆**：经验写入长期向量记忆

策略权重存储在 DashMap 中（无锁并发），多次运行后 Agent 会自动偏向历史上得分更高的策略。

## 长期记忆

支持两种模式：
- **Qdrant 模式**：启动 `docker run -d -p 6334:6334 qdrant/qdrant`，自动连接
- **内存模式**：Qdrant 不可用时自动降级（适合开发/测试）

向量嵌入支持：
- **Voyage AI**：设置 `VOYAGE_API_KEY` 环境变量
- **OpenAI**：使用 `text-embedding-3-small`（需 OpenAI API Key）
- **哈希嵌入**：免 API 的确定性嵌入（测试用）

## 运行测试

```bash
# 全部测试（37 个）
cargo test --workspace

# 特定 crate
cargo test -p scheduler
cargo test -p memory

# 集成测试
cargo test --test agent_loop
```

## 日志级别

```bash
RUST_LOG=error   # 仅错误
RUST_LOG=warn    # 警告及以上
RUST_LOG=info    # 关键步骤（推荐）
RUST_LOG=debug   # 完整调试信息
```

## 技术决策

| 决策 | 选择 | 原因 |
|---|---|---|
| 并发模型 | Tokio async/await | IO 密集型工具调用 |
| 权重存储 | DashMap（无锁） | 读多写少，低竞争 |
| 学习率 | AtomicU64 存 f64 bits | 无锁 f64 原子更新 |
| 工具注册 | Arc\<dyn Tool\> | 运行时动态注册 |
| DAG 执行 | 拓扑层并发 | 最大化并行度 |
| 向量记忆 | Qdrant + 内存降级 | 无需外部服务的开发体验 |
| 配置格式 | TOML | serde 原生支持 |
