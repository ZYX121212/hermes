# Hermes Agent

一个极简自进化 AI Agent 框架，用 Rust 实现。遵循五步反馈循环，每次执行后自动学习和优化策略权重。

```
Observe → Plan → Execute → Reflect → Evolve
   ↑________________________________________|
```

## 架构

```
hermes/
├── crates/
│   ├── agent-core/        # 核心 trait、数据结构、主循环
│   ├── hermess-agent/     # Agent 实现（subagent、MIMO、加密、输入守卫）
│   ├── evolution/         # 无锁策略权重进化引擎
│   ├── planner/           # LLM 驱动的任务分解（流式输出）
│   ├── memory/            # 短期工作记忆 + Qdrant 长期向量记忆
│   ├── tools/             # 动态工具注册 + 内置工具 + 浏览器 + 搜索
│   ├── reflector/         # 结果评分和错误归因
│   ├── llm/               # LLM 适配层（Anthropic / OpenAI 兼容 / DeepSeek）
│   ├── scheduler/         # DAG 拓扑排序并发执行
│   ├── tui/               # Ratatui 终端 UI + 用户设置持久化
│   ├── hermess-gateway/   # LLM 路由网关（多模型负载均衡、速率限制）
│   ├── hermess-platform/  # 平台适配器（飞书/企微/Discord/Slack/Telegram）
│   ├── hermess-finance/   # 金融数据工具（FTShare/TuShare/东方财富/腾讯/新浪）
│   ├── hermess-web/       # Web 会话管理
│   └── mcp/               # MCP (Model Context Protocol) 服务端/客户端
├── config/                # TOML 配置文件
├── src/main.rs            # CLI 入口
└── tests/                 # 集成测试
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

### 交互式配置（推荐）

```bash
# 启动配置向导，逐项设置 LLM、搜索、飞书、企业微信等
hermes configure

# 仅配置特定模块
hermes configure -s llm -s feishu
```

配置按四层优先级合并：**CLI flags > 环境变量 > settings.json > TOML config**

### 支持的 LLM 提供商

| 提供商 | 环境变量 | 配置值 |
|---|---|---|
| Anthropic | `ANTHROPIC_API_KEY` | `provider = "anthropic"` |
| OpenAI | `OPENAI_API_KEY` | `provider = "openai"` |
| DeepSeek | `DEEPSEEK_API_KEY` | `provider = "deepseek"` |
| 任意 OpenAI 兼容 API | — | `provider = "openai"` + `base_url` |

内置 **LiteLLM 模型目录集成**：配置 LiteLLM 端点后可自动获取各提供商最新模型列表，无需手动输入模型名。

### 运行

```bash
# 单次任务模式
cargo run --release -- --config config/deepseek.toml --task "你的任务描述"

# 交互模式 — 多轮对话
cargo run --release -- --interactive

# TUI 终端界面模式
cargo run --release -- --tui

# HTTP 服务器模式
cargo run --release -- --serve 8080

# Gateway 路由网关模式
cargo run --release -- --gateway

# MCP stdio 服务模式
cargo run --release -- --mcp-server

# 定时调度模式（cron 表达式）
cargo run --release -- --schedule "0 */6 * * *" --task "定时任务"

# 会话持久化
cargo run --release -- --task "..." --save session.json
cargo run --release -- --resume session.json

# 预加载知识库
cargo run --release -- --knowledge-base ./docs --task "..."
```

## 配置

### 配置向导 (`hermes configure`)

```
hermes configure           # 交互式逐项配置
hermes configure -s llm    # 仅 LLM 模块
hermes configure -s feishu # 仅飞书平台
```

向导涵盖：
- **LLM** — 提供商选择（Anthropic/OpenAI/DeepSeek）、API Key、模型选择（含 LiteLLM 自动获取）、Base URL
- **搜索** — Brave Search API 开关和 Key
- **金融** — 数据源选择（FTShare/TuShare/东方财富/腾讯/新浪）
- **飞书** — App ID、App Secret、Bot Open ID
- **企业微信** — Corp ID、Corp Secret、Agent ID

每个字段支持输入验证、变更预览、确认保存。配置保存到 `.hermess/settings.json`。

### 配置文件 (`config/default.toml`)

```toml
learning_rate = 0.1
working_memory_size = 100
max_concurrency = 10
max_step_retries = 3
max_replans = 1
compress_threshold = 20
compress_keep_ratio = 0.5
plugin_dirs = ["plugins"]

[llm]
provider = "deepseek"
model = "deepseek-chat"
max_tokens = 4096
base_url = "https://api.deepseek.com/v1"

[qdrant]
url = "http://localhost:6334"
collection = "hermes_memory"
embedding_dim = 1024

[search]
endpoint = "https://api.search.brave.com/res/v1/web/search"

[scorer]
success_weight = 0.6
latency_weight = 0.2
quality_weight = 0.2
latency_target_ms = 2000

[guard]
danger_mode = "ask"

# 平台配置（可选）
[feishu]
app_id = "cli_xxx"
app_secret = "xxx"
bot_open_id = "ou_xxx"

[wechat]
corp_id = "wwxxx"
corp_secret = "xxx"
agent_id = 1000002
```

### 环境变量

| 变量 | 用途 |
|---|---|
| `DEEPSEEK_API_KEY` | DeepSeek API Key |
| `OPENAI_API_KEY` | OpenAI API Key |
| `ANTHROPIC_API_KEY` | Anthropic API Key |
| `BRAVE_SEARCH_API_KEY` | Brave Search API Key |
| `FEISHU_APP_ID` / `FEISHU_APP_SECRET` / `FEISHU_BOT_OPEN_ID` | 飞书平台 |
| `WECHAT_CORP_ID` / `WECHAT_CORP_SECRET` / `WECHAT_AGENT_ID` | 企业微信 |
| `HERMESS_FINANCE_PROVIDER` | 金融数据源（ftshare/tushare/sina/eastmoney/tencent） |
| `HERMESS_TUSHARE_TOKEN` | TuShare Token |
| `LITELLM_URL` | LiteLLM 代理地址（自动获取模型列表） |
| `LOG_FORMAT=json` | JSON 格式日志 |

### API Key 优先级

1. CLI flags（`--api-key` / `--provider` / `--model`）
2. 环境变量（`DEEPSEEK_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`）
3. `.hermess/settings.json`（`hermes configure` 生成）
4. 配置文件 `[llm].api_key`

## 平台适配器

Hermes 支持多平台消息接入，统一通过 `InboundMessage` / `OutboundMessage` 模型：

| 平台 | 状态 | 功能 |
|---|---|---|
| **飞书 (Feishu/Lark)** | 已实现 | 消息收发、交互式卡片（审批按钮）、事件回调、Bot 菜单、reaction |
| **企业微信 (WeChat Work)** | 已实现 | 消息收发、模板卡片、点击/订阅/进入事件回调 |
| **Discord** | 骨架 | — |
| **Slack** | 骨架 | — |
| **Telegram** | 骨架 | — |

配置通过 4 层优先级（CLI > env > settings.json > TOML）统一管理。

## 内置工具

| 工具 | 用途 |
|---|---|
| `bash` | 执行 shell 命令（含危险操作守卫） |
| `web_search` | Brave Search API 网页搜索 |
| `read_file` | 文件读取 |
| `write_file` | 文件写入 |
| `reply` | 文本回复 |
| `financial` | 金融数据查询（股票/指数/基金，多源自动 failover） |
| `browser_click` / `browser_fill` / `browser_exec` / `browser_screenshot` | 浏览器自动化 |
| `code_exec` | 代码执行沙箱 |
| `vision` | 图像分析 |
| 插件系统 | `plugins/` 目录自动发现 Shell/Script 插件 |

## CLI 选项

```
Usage: hermes [OPTIONS] [COMMAND]

Commands:
  configure  交互式配置向导

Options:
  -c, --config <CONFIG>        配置文件路径 [default: config/default.toml]
  -p, --profile <PROFILE>      配置预设（dev/prod）
  -t, --task <TASK>            单次任务
  -i, --interactive            交互模式（多轮对话）
      --tui                    启动 Terminal UI
      --serve <PORT>           启动 HTTP 服务器
      --gateway                启动 LLM 路由网关
      --mcp-server             MCP stdio 服务模式
      --schedule <CRON>        Cron 定时调度
      --save <PATH>            退出时保存会话
      --resume <PATH>          恢复之前的会话
      --knowledge-base <DIR>   预加载知识库目录

  LLM:
      --api-key <KEY>          API Key
      --provider <NAME>        提供商（anthropic/openai/deepseek）
      --model <NAME>           模型名
      --base-url <URL>         API Base URL
      --max-tokens <N>         最大 Token 数

  搜索:
      --search-api-key <KEY>   Brave Search API Key

  平台:
      --feishu-app-id <ID>            飞书 App ID
      --feishu-app-secret <SECRET>    飞书 App Secret
      --feishu-bot-open-id <ID>       飞书 Bot Open ID
      --wechat-corp-id <ID>           企业微信 Corp ID
      --wechat-corp-secret <SECRET>   企业微信 Corp Secret
      --wechat-agent-id <ID>          企业微信 Agent ID

  调优:
      --learning-rate <FLOAT>    学习率 (0.0–1.0)
      --max-concurrency <N>      最大并发
      --danger-mode <MODE>       危险命令策略 (ask/skip/deny)
      --max-step-retries <N>     步骤最大重试
      --max-replans <N>          最大重规划次数
      --compress-threshold <N>   上下文压缩阈值
```

## 运行模式

| 模式 | 命令 | 说明 |
|---|---|---|
| 单次任务 | `--task "..."` | 执行一次后退出 |
| 交互对话 | `--interactive` | 多轮对话，stdin 输入 |
| TUI | `--tui` | Ratatui 终端界面，实时进度 |
| HTTP 服务器 | `--serve 8080` | `POST /agent/run` API |
| Gateway | `--gateway` | LLM 路由网关（多模型负载均衡） |
| MCP Server | `--mcp-server` | Model Context Protocol stdio 服务 |
| Cron 调度 | `--schedule "0 */6 * * *"` | 定时自动执行 |

## TUI 终端界面

运行 `hermes --tui` 启动全功能终端 UI，支持实时流式输出、设置管理与多面板交互。

### 面板布局

```
┌─────────────────────────────────────┬──────────────────┐
│  Plan / Execution 主内容区           │  Evolution 进化  │
│  · 计划步骤列表 & 展开详情           │  · 策略权重趋势  │
│  · 流式输出实时追加                  │  · 学习率曲线    │
│  · [←/→] 切换 Plan/Execution 视图   │  · 历史得分      │
├─────────────────────────────────────┴──────────────────┤
│  MiniLog  最近事件（All / Errors 切换）                 │
├─────────────────────────────────────────────────────────┤
│  ▶ Input >  任务输入框（Enter 提交 · Shift+Enter 换行） │
└─────────────────────────────────────────────────────────┘
```

### 快捷键速查

| 按键 | 动作 |
|---|---|
| `i` | 开始输入（Agent 空闲时） |
| `Enter` | 提交任务 |
| `Shift+Enter` | 输入框内换行 |
| `p` | 取消当前 Agent 操作 |
| `Tab` / `Shift+Tab` | 顺时针/逆时针切换焦点面板 |
| `↑↓` / `j k` | 滚动行 |
| `PgUp` / `PgDn` | 翻页 |
| `Home` / `End` | 跳至顶部/底部 |
| `[` / `]` | 调整左右分栏比例 |
| `h` / `F1` | 显示/隐藏帮助面板（面板内同样可滚动） |
| `s` / `F2` | 打开设置面板（LLM / 搜索 / 金融 / 飞书 / 主题） |
| `f` | 日志面板：切换 All / Errors 过滤 |
| `q` / `Ctrl+C` | 退出（`q` 仅在空闲时可用） |
| `Ctrl+Y` | 复制聚焦内容到剪贴板 |
| `Ctrl+S` | 导出对话到文件 |
| `/` | 进入搜索模式 |
| `n` / `N` | 下一个/上一个搜索匹配 |

### 斜杠命令

在输入框中以 `:` 触发命令：

| 命令 | 说明 |
|---|---|
| `:new` | 清空会话，开始新任务 |
| `:diff` | 显示 `git diff --stat HEAD` |
| `:status` | 显示 Agent 运行状态摘要 |
| `:usage` | 显示 Token / API 用量统计 |
| `:cron` | 管理定时任务 |
| `:memory` | 查看/清除长期记忆 |
| `:personality` | 切换 Agent 性格预设 |
| `:debug` | 切换调试日志级别 |
| `:compress` | 立即压缩上下文 |

### 最小终端要求

TUI 模式要求终端宽度 ≥ 40 列、高度 ≥ 10 行。低于此尺寸时界面显示友好提示而非崩溃。

---

## 进化机制

每次执行后，Agent 自动学习：

1. **评分**：根据成功率、延迟、输出质量计算得分 [-1, 1]
2. **归因**：失败时调用 LLM 分析原因
3. **更新权重**：`weight += score × learning_rate`
4. **衰减**：学习率随时间递减 `lr_t = lr_0 / sqrt(n + 1)`
5. **记忆**：经验写入长期向量记忆

策略权重存储在 DashMap 中（无锁并发），多次运行后 Agent 会自动偏向历史上得分更高的策略。

## 金融数据

内置多源 failover 链：`TuShare → FTShare → 东方财富 → 腾讯 → 新浪`

```bash
# 通过环境变量指定
HERMESS_FINANCE_PROVIDER=tushare HERMESS_TUSHARE_TOKEN=xxx cargo run -- --task "查询贵州茅台股价"

# 通过 settings.json（hermes configure 设置）
```

## 长期记忆

- **Qdrant 模式**：`docker run -d -p 6334:6334 qdrant/qdrant`，自动连接
- **内存模式**：Qdrant 不可用时自动降级

向量嵌入：Voyage AI / OpenAI / 哈希嵌入（测试用）

## 运行测试

```bash
# 全量测试
cargo test --workspace

# 特定 crate
cargo test -p scheduler
cargo test -p tui
cargo test -p hermess-platform
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
| 配置优先级 | CLI > env > settings.json > TOML | 符合 12-Factor App |
| 平台适配 | trait PlatformAdapter | 统一消息模型，多平台复用 |
