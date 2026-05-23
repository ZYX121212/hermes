# Hermes Agent — 可见性优化设计

日期：2026-05-23

## 目标

消除"对话不可见的后果"——确保 Agent 运行过程中的所有关键状态变更、错误、降级行为都可见、可追踪。不改动功能和架构，只修复日志和错误传播。

## 日志分层标准

| 级别 | 使用场景 |
|------|----------|
| ERROR | 数据损坏、不可恢复的状态、用户必须知道的问题 |
| WARN | 降级行为、重试、事件通道断开、持久化失败 |
| INFO | 阶段切换、进化权重变更、LLM 调用开始/完成、策略选择 |
| DEBUG | 工具耗时、请求参数、内部状态细节、SSE chunk 解析 |

## 修改清单

### src/main.rs（5 处）

- `emit()` 通道断开 → `warn!("Event channel closed, TUI may have disconnected")`
- `load_from_file` → 保留原始错误信息，区分"文件不存在"和"解析失败"
- API key 未设置 → `warn!("No {provider} API key configured")`
- Agent 启动 → `info!("Hermes agent starting: provider={}, model={}, config={}")`
- 退出码：保存演化状态失败时返回非零

### crates/agent-core（3 处）

- `agent.rs` 默认 `run_loop` → 各阶段前后 `info!`
- `runner.rs` → `info!("Starting agent loop: task={}, max_iter={}")`
- `context.rs` Ctrl+C 注册失败 → `error!`

### crates/llm（6 处）

- Anthropic/OpenAI 响应结构异常 → `error!("Unexpected API response structure")` + 保留原始 JSON
- 嵌入 API 失败 → `warn!("Embedding API failed, using zero-vector fallback")`
- 请求开始/完成 → `info!("LLM request: provider={}, model={}, tokens={}")`
- SSE 解析失败 → `debug!`（高频操作，不污染 INFO 日志）

### crates/evolution（3 处）

- 权重更新 → `info!("Strategy {id} weight: {old:.4} -> {new:.4}, delta={delta:.4}, lr={lr:.4}")`
- 记忆存入 → 检查 JoinHandle，panic 时 `warn!`
- 文件 I/O 详情 → `debug!`

### crates/planner（3 处）

- `best_strategy()` 返回 None → `info!("No strategy data available, using default")`
- 工具描述序列化失败 → `error!("Failed to serialize tool descriptions")`
- 重试 → `warn!("Plan parse failed, retrying ({}/{})")`

### crates/scheduler（2 处）

- 错误分类：tool_failed、tool_not_found、internal_error 使用不同日志模式
- 信号量获取失败 → 返回错误而非 panic

### crates/tools（2 处）

- web_search 响应体读取失败 → `warn!` + `success: false`
- bash/file 保留 OS 错误类型到日志

### crates/memory（3 处）

- Qdrant 连接失败降级 → `info!` 明确降级原因
- 无效 UUID → `warn!`
- 嵌入零向量回退 → 升级为 `warn!`

### crates/tui（2 处）

- 渲染失败 → `error!("Terminal draw failed")` + 状态标记
- JoinHandle 丢弃 → 检查 panic 并 `error!`

## 不改动的部分

- Trait 签名保持不变
- CLI/TUI 交互流程不变
- 调度器、规划器、评分器核心逻辑不变
- Qdrant 连接与降级逻辑不变

## 自检清单

- [ ] 所有 `let _ =` 错误丢弃点都有对应的日志
- [ ] 所有降级行为都有 WARN 级别日志
- [ ] 所有阶段切换在默认循环中都有 INFO 日志
- [ ] 进化权重更新在默认日志级别可见
- [ ] 无新增 panic 点
- [ ] 无 API 签名变更
