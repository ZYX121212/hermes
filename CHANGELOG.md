# Changelog

所有重要变更记录。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [1.0.0] - 2026-06-10

### 新增

- **小窗口保护**：终端宽度 < 40 列或高度 < 10 行时渲染友好提示，不再崩溃
- **Help 面板滚动**：`↑↓ j/k PgUp PgDn Home` 键滚动帮助文档
- **`/diff` 命令实装**：执行 `git diff --stat HEAD`，输出显示在弹窗中
- **100 轮压力测试**：`crates/tui/tests/stress_100.rs` 模拟 100 轮完整任务循环
- **公共 API 包装器**：`handle_event_pub`、`submit_tui_input_pub` 等，支持集成测试

### 优化

- **60fps 渲染**：事件轮询间隔 33ms → 16ms（`RENDER_POLL_MS` 常量）
- **流式缓冲区限速**：`streaming_buffer` 与 `summary_streaming_buffer` 超过 50KB 时按行修剪，避免大输出下帧率下降
- **渲染常量**：新增 `MIN_WIDTH=40`、`MIN_HEIGHT=10`、`RENDER_POLL_MS=16`

### 修复

- **p 键取消后死锁**：取消后 `stop_flag` 自动重置，用户可继续提交下一条任务
- **安全注释**：`keybindings.rs` 中唯一 `unwrap()` 添加 `// SAFETY:` 说明

### 测试

- 单元测试从 388 条增至 **564 条**（lib），新增覆盖：
  - `render_scrollbar` 边界（空内容、精确适配、溢出、最大滚动）
  - `FocusedPanel::next/prev` 双向完整循环
  - `submit_tui_input`、`begin_next_task_input` 全状态重置
  - `handle_help_overlay_key` 所有按键行为
  - `toggle_evolution_section`、`scroll_to_top/bottom`、`page_scroll_focused`
  - `dispatch_slash_command`：`/new /diff /status /usage /cron /memory /personality /debug`
  - `handle_event` 全事件管线（TurnStarted → Observing → Planning → Executing → Reflecting → Evolving → Idle）
  - 50KB 流式缓冲区修剪验证
  - KanbanItem、TuiInput 构造、AgentPhase::main_split_ratio、strip_html 实体

---

## [0.1.0] - 2026-06-08

### 新增

- **多平台消息适配器**：飞书、企业微信、Discord、Slack、Telegram
- **LLM 网关路由**：多模型支持，智能路由，负载均衡
- **LiteLLM 模型目录**：自动拉取可用模型列表
- **金融数据层**：富时、TuShare、新浪、东方财富、腾讯数据源
- **自我进化引擎**：基于反馈的提示词优化和参数调优
- **长期记忆系统**：向量嵌入存储 + 内容去重
- **工具系统**：Bash、文件读写、浏览器自动化、网页搜索、Python 代码执行
- **安全守卫**：危险命令检测与审批（Deny/Ask/Auto 三级策略）
- **MCP 协议支持**：Model Context Protocol 集成
- **TUI 终端界面**：多面板实时交互，设置管理
- **Web 管理端**：HTTP API、WebSocket、健康检查
- **配置向导**：交互式引导配置流程
- **定时任务调度**：cron 风格定时执行
- **插件系统**：TOML 声明式扩展，Shell/Script 双模式
- **反思与归因**：执行结果分析与反馈收集
- **配置热重载**：飞书/企业微信配置实时同步

### 安全

- 危险命令检测（rm -rf、sudo、chmod 777 等 27 种模式）
- 插件解释器白名单校验
- 浏览器沙箱默认启用
- API Key 脱敏显示
- 不再硬编码任何密钥
