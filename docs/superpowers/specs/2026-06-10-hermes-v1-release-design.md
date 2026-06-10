# Hermes v1.0 商业发布设计规格

**日期**: 2026-06-10  
**范围**: TUI 商业发布质量 — 稳定化、测试体系、功能补齐、发布物料  
**状态**: 已批准，进入实现计划阶段

---

## 背景与目标

Hermes 是一个基于 Rust 的 AI Agent TUI 终端框架，包含 15 个 crate。当前状态：
- 788 个测试，0 失败
- 15 个斜杠命令已实现，`/diff` 为占位符
- 交互逻辑 bug（Tab、Esc、Enter、p键）已修复
- 存在 89 个生产代码 `unwrap()` 有崩溃风险

**发布目标**：商业产品级别，需要零崩溃、完整测试、文档和 CI/CD。

---

## 方案：分阶段稳固推进

### 阶段 1：稳定化（Stabilization）

#### 1a. 零 panic 保证
扫描 `crates/tui/src/`（非测试代码）中所有 `unwrap()`，按风险分级处理：

| 风险等级 | 处理方式 |
|---------|---------|
| 高（用户输入路径、索引越界） | `.get()` + 日志 + 默认值 |
| 中（初始化路径） | `unwrap_or_default()` / `unwrap_or_else` |
| 低（编译期已知安全） | 保留并添加 `// SAFETY:` 注释说明原因 |

**目标**：生产路径中零不安全 `unwrap()`。

#### 1b. 小窗口安全渲染
在 `render_app` 最顶部添加最小尺寸检测：

```
条件: 终端宽度 < 40 列 OR 高度 < 10 行
行为: 全屏渲染友好提示，不进行正常布局
提示: "请调整终端大小 (当前: {w}×{h}，最小要求: 40×10)"
```

避免 `Rect` 溢出和布局计算崩溃。

#### 1c. Help 面板滚动支持
当帮助内容超出窗口高度时，添加 `↑↓` 滚动（当前为固定内容，可能在小窗口下截断）。
新增 `help_scroll: u16` 字段到 `TuiAppState`。

#### 1d. Evolution 面板空权重越界防护
`evo_scroll` 超出实际内容长度时做 `saturating` 夹紧。

---

### 阶段 2：测试体系（Test System）

#### 2a. 100 轮模拟压力测试
新文件：`crates/tui/tests/stress_100.rs`

```
MockAgent: 实现 HermesAgent trait，收到任务立即发射 TurnStarted + SummaryReady 事件并返回 Ok(())
MockContext: 100次迭代限制

测试流程（循环 100 次）:
  1. submit_tui_input(state, format!("任务{round}")) → submitted = Some(text)
  2. 驱动事件: handle_event(TurnStarted{turn: round})
  3. 驱动事件: handle_event(SummaryReady{summary: "done"})
  4. outer_loop 模拟: agent_done = true, awaiting.store(true)
  5. 断言: agent_done=true, phase=Idle, 无 panic
  6. begin_next_task_input → awaiting.store(true), 进入下一轮

最终断言:
  - input_history.len() == 100
  - log_entries.len() <= 200 (自动修剪)
  - state 一致性: phase=Idle, agent_done=true
```

#### 2b. 补充单元测试（388 → 目标 560+）

| 区域 | 新增测试数 |
|-----|----------|
| `render_scrollbar` 边界（空、满、溢出） | 8 |
| `FocusedPanel::next/prev` 完整双向循环 | 6 |
| `submit_tui_input` 历史记录 + 50 条上限 | 6 |
| 搜索模式完整流程（开启→搜索→导航→清除） | 10 |
| 多行输入 `\n`、光标位置、Shift+Enter | 8 |
| `/diff` 新实现（git 仓库 + 非 git 仓库） | 4 |
| `Ctrl+Tab`、`[`、`]` 快捷键状态变化 | 6 |
| 设置面板所有字段类型（Toggle/Dropdown/Text） | 8 |
| `begin_next_task_input` 状态重置 | 4 |
| `p` 键取消后 stop_flag 重置 + 继续输入 | 6 |
| 100 轮压力测试关键断言 | 12 |
| 小窗口保护渲染路径 | 4 |

**总计**: +82 测试，合计 ~470（lib tests）+ 100 轮集成 = **~570+**

#### 2c. 质量门禁
每次提交必须通过：
- `cargo test --workspace` 0 失败
- `cargo clippy -- -D warnings` 0 警告
- `cargo fmt --check` 通过

---

### 阶段 3：功能补齐（Feature Completion）

#### 3a. `/diff` 命令实现
- 调用 `std::process::Command::new("git").args(["diff", "--stat", "HEAD"])`
- 输出显示在 `slash_command_popup`（可滚动）
- 若执行失败（非 git 仓库）显示：`"当前目录不是 git 仓库，无法获取 diff"`
- 若无变更显示：`"无变更 (git diff --stat HEAD 为空)"`

#### 3b. 渲染性能优化（目标 60fps）
- 将 `crossterm::event::poll(Duration::from_millis(33))` 改为 `poll(16ms)`
- 添加**帧脏标记**：`dirty: AtomicBool`，agent 事件或用户输入时置 `true`，无变化时跳过重绘
- `streaming_buffer` 超过 50KB 时只向渲染层传递最后 1000 行，避免大文本帧率下降
- `summary_streaming_buffer` 同上

#### 3c. CI/CD（GitHub Actions）
新文件 `.github/workflows/ci.yml`：
```yaml
name: CI
on:
  push:
    branches: [main, dev]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
  
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: components: clippy, rustfmt
      - run: cargo clippy -- -D warnings
      - run: cargo fmt --check
  
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
```

---

### 阶段 4：发布物料（Release Artifacts）

#### 4a. README.md（根目录）
包含：
1. 项目简介 + 徽章（CI 状态、Rust 版本、License）
2. 功能截图（ASCII art 截图或文字演示）
3. 安装方法（`cargo install` 或从 Release 下载）
4. 快速开始（5 步入门）
5. 配置说明（环境变量 + `~/.hermes/config.toml`）
6. 快捷键速查表
7. 贡献指南

#### 4b. CHANGELOG.md
遵循 [Keep a Changelog](https://keepachangelog.com) 格式，记录 v1.0.0 所有变更。

#### 4c. Cargo.toml 元数据更新
所有 crate 的 `Cargo.toml`：
```toml
version = "1.0.0"
description = "..."
license = "MIT"
repository = "https://github.com/..."
keywords = [...]
categories = [...]
```

#### 4d. docs/configuration.md
完整配置项参考：LLM 设置、搜索、金融数据源、飞书集成、主题配置。

---

## 成功标准

| 标准 | 验证方式 |
|-----|---------|
| 零生产 panic | `grep -r 'unwrap()' crates/*/src/ --include="*.rs" \| grep -v test` → 0 条高风险结果 |
| 测试全绿 | `cargo test --workspace` 0 失败 |
| 100 轮压力测试通过 | `stress_100.rs` 全部断言通过 |
| 小窗口安全 | 40×10 以下不崩溃，显示友好提示 |
| CI 通过 | GitHub Actions 所有 job 绿色 |
| 文档完整 | README、CHANGELOG、配置文档齐全 |
| 60fps 渲染 | `poll(16ms)` + 脏标记，大输出场景无明显卡顿 |

---

## 不在本次范围内

- 语音输入（Ctrl+B）：依赖外部 STT 库，复杂度过高
- 多 Profile 切换（`--profile`）：需要 CLI + 配置系统重构
- MCP 服务器新功能：单独立项
- 移动端 / Web 端：超出范围
