# 贡献指南

感谢你对 Hermes Agent 的关注！欢迎任何形式的贡献。

## 行为准则

- 保持友善和尊重
- 建设性批评，不人身攻击
- 关注问题本身，不扩大讨论范围

## 如何贡献

### 报告 Bug

1. 在 [Issues](https://github.com/ZYX121212/hermes/issues) 中搜索，确认未被报告过
2. 使用 Bug Report 模板创建 issue
3. 提供：环境信息（OS、Rust 版本）、复现步骤、预期行为、实际行为、日志

### 提交功能请求

1. 在 Issues 中搜索，确认未被提出过
2. 使用 Feature Request 模板
3. 描述：使用场景、期望行为、为什么现有功能不能满足

### 提交代码

1. **Fork** 本仓库
2. 创建特性分支：`git checkout -b feature/your-feature`
3. 遵循代码风格：
   - `cargo fmt` 格式化代码
   - `cargo clippy` 通过无警告
   - 为新功能添加测试
   - 为公开 API 添加文档注释
4. 确保所有测试通过：`cargo test --workspace`
5. 提交时使用清晰的中文或英文 commit message
6. 推送并创建 Pull Request

## PR 审核标准

- CI 全部通过（fmt、clippy、test、build）
- 新功能有对应的测试覆盖
- 公开 API 有文档注释
- 无安全漏洞（禁止硬编码密钥、命令注入等）
- 不引入 `unsafe` 代码（需要充分理由并在 PR 中明确说明）

## 项目结构

```
crates/
├── agent-core/       # Agent 核心抽象（Context、Session、Agent trait）
├── hermess-agent/    # HermesAgent 实现（reAct 循环）
├── evolution/        # 自我进化引擎
├── planner/          # 规划器
├── memory/           # 长期记忆（向量嵌入 + 去重）
├── tools/            # 工具系统（bash、文件、浏览器、搜索、金融）
├── reflector/        # 反思与归因
├── llm/              # LLM 适配器（DeepSeek、OpenAI、Anthropic）
├── scheduler/        # 定时任务调度
├── hermess-gateway/  # 多模型网关与路由
├── hermess-platform/ # 消息平台适配（飞书、企微、Discord 等）
├── hermess-finance/  # 金融数据层
├── hermess-web/      # Web 管理端
├── mcp/              # MCP 协议支持
└── tui/              # 终端用户界面
```

## 开发环境

```bash
# 配置环境变量
export DEEPSEEK_API_KEY="your-key"

# 运行配置向导（推荐）
cargo run -- configure

# 构建
cargo build --release

# 运行测试
cargo test --workspace

# 启动 TUI 模式
cargo run
```

## 联系方式

- Issue tracker: https://github.com/ZYX121212/hermes/issues
- 讨论区: https://github.com/ZYX121212/hermes/discussions
