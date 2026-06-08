# Changelog

所有重要变更记录。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

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
