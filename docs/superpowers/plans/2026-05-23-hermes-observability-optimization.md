# Hermes Agent 可见性优化 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除静默错误丢弃和关键状态变更不可见的问题，统一日志分层标准（ERROR/WARN/INFO/DEBUG）。

**Architecture:** 逐 crate 修改，不改 trait 签名和核心逻辑，只在错误丢弃点、状态变更点、降级行为处添加 tracing 日志，并将 5 处潜在的 panic 转为安全处理。

**Tech Stack:** Rust, tokio, tracing, anyhow

---

### Task 1: src/main.rs — 事件发送、API key、启动日志、退出码

**Files:**
- Modify: `src/main.rs:351-355, 457-459, 399-440, 554-557`

- [ ] **Step 1: `emit()` 添加通道断开警告**

将第 353 行：
```rust
let _ = tx.send(event);
```
改为：
```rust
if tx.send(event).is_err() {
    tracing::warn!("Event channel closed — TUI observer may have disconnected");
}
```

- [ ] **Step 2: `load_from_file` 保留原始错误信息**

将第 457-459 行：
```rust
.unwrap_or_else(|_| {
    tracing::info!("No previous evolution state found, starting fresh");
    evolution::EvolutionEngine::new(cfg.learning_rate, Arc::clone(&memory))
}),
```
改为：
```rust
.unwrap_or_else(|e| {
    if e.to_string().contains("No such file") || e.to_string().contains("entity not found") {
        tracing::info!("No previous evolution state found, starting fresh");
    } else {
        tracing::warn!("Failed to load evolution state ({e}), starting fresh");
    }
    evolution::EvolutionEngine::new(cfg.learning_rate, Arc::clone(&memory))
}),
```

- [ ] **Step 3: API key 缺失时使用 warn! 日志**

在第 399-440 行的 LLM 适配器构造代码中，修改两个 API key 获取位置：

在第 402-404 行（openai/deepseek 分支），将：
```rust
let key = if cfg.llm.api_key.is_empty() {
    std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
        .unwrap_or_default()
```
改为：
```rust
let key = if cfg.llm.api_key.is_empty() {
    let k = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
        .unwrap_or_default();
    if k.is_empty() {
        tracing::warn!("No OpenAI/DeepSeek API key configured — API calls will fail");
    }
    k
```

在第 429-431 行（anthropic 分支），将：
```rust
let key = if cfg.llm.api_key.is_empty() {
    std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
```
改为：
```rust
let key = if cfg.llm.api_key.is_empty() {
    let k = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    if k.is_empty() {
        tracing::warn!("No Anthropic API key configured — API calls will fail");
    }
    k
```

- [ ] **Step 4: Agent 启动时添加 info! 日志**

在第 522 行（`// ── Run ──` 注释块之前）插入：
```rust
tracing::info!(
    "Hermes agent starting: provider={}, model={}, config={}, interactive={}, tui={}",
    cfg.llm.provider,
    cfg.llm.model,
    cli.config,
    cli.interactive,
    cli.tui,
);
```

- [ ] **Step 5: 演化状态保存失败时使用非零退出码**

将第 554-557 行：
```rust
if let Err(e) = evolution_handle.save_to_file(".hermes_evolution.json") {
    tracing::warn!("Failed to save evolution state: {e}");
}

tracing::info!("Hermes Agent stopped.");
Ok(())
```
改为：
```rust
let mut exit_code = 0;
if let Err(e) = evolution_handle.save_to_file(".hermes_evolution.json") {
    tracing::warn!("Failed to save evolution state: {e}");
    exit_code = 1;
}

tracing::info!("Hermes Agent stopped.");
if exit_code != 0 {
    std::process::exit(exit_code);
}
Ok(())
```

- [ ] **Step 6: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "fix(main): add warn logs for event channel, API key, evolution load errors, and non-zero exit on save failure"
```

---

### Task 2: crates/agent-core — 默认 run_loop 日志、runner 日志、Ctrl+C 错误

**Files:**
- Modify: `crates/agent-core/src/agent.rs:19-31`
- Modify: `crates/agent-core/src/runner.rs:7-10`
- Modify: `crates/agent-core/src/context.rs:33,53`

- [ ] **Step 1: 默认 run_loop 添加阶段日志**

将 `agent.rs` 第 19-31 行的 `run_loop` 改为：
```rust
async fn run_loop(&mut self, ctx: Context) -> Result<()> {
    loop {
        tracing::info!("Turn starting: observe phase");
        let obs = self.observe(&ctx).await?;
        if ctx.should_stop() {
            tracing::info!("Stop signaled after observe, exiting loop");
            break;
        }
        tracing::info!("Plan phase");
        let plan = self.plan(obs).await?;
        tracing::info!(steps = plan.steps.len(), "Execute phase");
        let result = self.execute(plan).await?;
        tracing::info!(success = result.success, duration_ms = result.duration_ms, "Reflect phase");
        let insight = self.reflect(&result).await?;
        tracing::info!(score = insight.score, strategy = %insight.strategy_id, "Evolve phase");
        self.evolve(insight).await?;
        if ctx.should_stop() {
            break;
        }
    }
    Ok(())
}
```

- [ ] **Step 2: runner.rs 添加上下文日志**

将 `runner.rs` 第 7-10 行改为：
```rust
pub async fn run_agent(mut agent: impl HermesAgent, ctx: Context) -> Result<()> {
    let task_desc = ctx.task().unwrap_or("(interactive)");
    tracing::info!(task = task_desc, "Hermes agent starting...");
    agent.run_loop(ctx).await
}
```

- [ ] **Step 3: Ctrl+C 注册失败改为 error! 日志**

在 `context.rs` 的 `new()`（第 32-36 行）和 `interactive()`（第 52-56 行）中，将 `tokio::spawn(async move { ... })` 改为保存 `JoinHandle` 并检查错误。

`new()` 中，将第 32-36 行：
```rust
let flag = Arc::clone(&ctx.stop_flag);
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("received Ctrl-C, signalling stop");
    flag.store(true, Ordering::Relaxed);
});
```
改为：
```rust
let flag = Arc::clone(&ctx.stop_flag);
tokio::spawn(async move {
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!("received Ctrl-C, signalling stop");
            flag.store(true, Ordering::Relaxed);
        }
        Err(e) => {
            tracing::error!("Failed to register Ctrl-C handler: {e}");
        }
    }
});
```

`interactive()` 中同样修改（第 52-56 行）。

- [ ] **Step 4: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add crates/agent-core/src/agent.rs crates/agent-core/src/runner.rs crates/agent-core/src/context.rs
git commit -m "fix(agent-core): add info logs for phase transitions and error log for Ctrl-C failure"
```

---

### Task 3: crates/llm — 异常响应日志、嵌入降级 warn、SSE 解析日志

**Files:**
- Modify: `crates/llm/src/anthropic.rs:73-76, 100, 111-118`
- Modify: `crates/llm/src/openai.rs:83-86, 109, 136-141, 148, 151`
- Modify: `crates/llm/src/stream.rs:65, 74`

- [ ] **Step 1: anthropic.rs — 响应结构异常时打印 ERROR 日志**

将第 73-76 行：
```rust
let text = body["content"][0]["text"]
    .as_str()
    .unwrap_or("")
    .to_string();
```
改为：
```rust
let text = body["content"][0]["text"]
    .as_str()
    .unwrap_or_else(|| {
        tracing::error!(
            "Unexpected Anthropic API response structure: {}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| "(unprintable)".into())
        );
        ""
    })
    .to_string();
```

- [ ] **Step 2: anthropic.rs — 嵌入零向量回退升级为 warn!**

将第 111-118 行 `embed()` 方法中的 `tracing::debug!` 改为 `tracing::warn!`：
```rust
tracing::warn!(
    "AnthropicAdapter: embed() returning zero vector (use VoyageEmbedder instead)"
);
```

- [ ] **Step 3: openai.rs — 响应结构异常时打印 ERROR 日志**

将第 83-86 行：
```rust
let text = body["choices"][0]["message"]["content"]
    .as_str()
    .unwrap_or("")
    .to_string();
```
改为：
```rust
let text = body["choices"][0]["message"]["content"]
    .as_str()
    .unwrap_or_else(|| {
        tracing::error!(
            "Unexpected OpenAI API response structure: {}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| "(unprintable)".into())
        );
        ""
    })
    .to_string();
```

- [ ] **Step 4: openai.rs — 嵌入 API 失败改为 warn! 日志**

将第 136-141 行的 `tracing::debug!` 改为 `tracing::warn!`，并在消息中保留 HTTP 状态码：
```rust
if !status.is_success() {
    tracing::warn!(
        "Embedding endpoint returned {status} — provider may not support embeddings, using zero vector"
    );
    return Ok(vec![0.0_f32; 1024]);
}
```

- [ ] **Step 5: openai.rs — 嵌入响应解析失败值使用 warn!**

第 148 行 `v.as_f64().unwrap_or(0.0) as f32` 中，嵌入值解析失败改为 warn（但要统计防止洪水）：
```rust
.map(|v| {
    let val = v.as_f64().unwrap_or(0.0) as f32;
    if v.as_f64().is_none() {
        tracing::warn!("Embedding value is not a number: {v}");
    }
    val
})
```
注意：这里只需在 `v.as_f64()` 为 None 时打一次日志，使用 `_` 计数变量防止洪水过于复杂。改为不修改此处的直接逻辑，而是在 `.unwrap_or_default()` 外层加一个日志就更合适。实际上这里应该更简单 —— 直接改为：
```rust
.map(|arr| {
    arr.iter()
        .map(|v| v.as_f64().map(|x| x as f32).unwrap_or_else(|| {
            tracing::warn!("Non-numeric embedding value: {v}");
            0.0
        }))
        .collect()
})
```

- [ ] **Step 6: openai.rs — 嵌入数组为空时 warn!**

将第 151 行的 `.unwrap_or_default()` 改为包含日志：
```rust
.unwrap_or_else(|| {
    tracing::warn!("Embedding response data array is empty or missing — using empty vector");
    Vec::new()
});
```

- [ ] **Step 7: stream.rs — SSE 解析失败使用 debug!**

将第 65 行：
```rust
Err(_) => continue,
```
改为：
```rust
Err(e) => {
    tracing::debug!("SSE data line parse failed: {e}, skipping chunk");
    continue;
}
```

- [ ] **Step 8: stream.rs — 非 UTF8 字节使用 debug!**

这段在 buffer 追加时，`from_utf8_lossy` 在遇到非 UTF8 字节会返回 Cow::Owned。我们改为在非 UTF8 时打印 debug 日志：

将第 73-76 行：
```rust
Poll::Ready(Some(Ok(chunk))) => {
    self.buffer
        .push_str(&String::from_utf8_lossy(&chunk));
}
```
改为：
```rust
Poll::Ready(Some(Ok(chunk))) => {
    match String::from_utf8(chunk.to_vec()) {
        Ok(s) => self.buffer.push_str(&s),
        Err(_) => {
            tracing::debug!("Non-UTF8 bytes in stream, using lossy conversion");
            self.buffer.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
}
```

- [ ] **Step 9: LLM 调用添加 info! 日志**

在 `anthropic.rs` 的 `complete()` 方法开头（第 49 行之后）添加：
```rust
tracing::info!(
    provider = "anthropic",
    model = %self.model,
    prompt_len = prompt.len(),
    "LLM completion request"
);
```

在 `openai.rs` 的 `complete()` 方法开头（第 56 行之后）添加：
```rust
tracing::info!(
    provider = "openai",
    model = %self.model,
    prompt_len = prompt.len(),
    "LLM completion request"
);
```

- [ ] **Step 10: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 11: Commit**

```bash
git add crates/llm/src/anthropic.rs crates/llm/src/openai.rs crates/llm/src/stream.rs
git commit -m "fix(llm): add error/warn logs for unexpected API responses, embedding fallbacks, and SSE parse failures"
```

---

### Task 4: crates/evolution — 权重更新 INFO、JoinHandle panic 检查、file I/O debug

**Files:**
- Modify: `crates/evolution/src/engine.rs:63-69, 85-89`

- [ ] **Step 1: 权重更新升级为 info!**

将第 63-69 行：
```rust
tracing::debug!(
    strategy = %insight.strategy_id,
    score = insight.score,
    lr = lr_t,
    delta = delta,
    "evolution update"
);
```
改为：
```rust
// Read old weight before update
let old_weight = self.strategy_weights
    .get(&insight.strategy_id)
    .map(|w| *w);
let strategy_id = insight.strategy_id.clone();

// 3. Lock-free strategy weight update with clamping
self.strategy_weights
    .entry(strategy_id.clone())
    .and_modify(|w| *w = clamp(*w + delta, -10.0, 10.0))
    .or_insert(clamp(delta, -10.0, 10.0));

let new_weight = self.strategy_weights
    .get(&strategy_id)
    .map(|w| *w)
    .unwrap_or(0.0);

tracing::info!(
    strategy = %strategy_id,
    old = old_weight.unwrap_or(0.0),
    new = new_weight,
    delta,
    lr = lr_t,
    score = insight.score,
    "evolution update"
);
```

注意：需要删除原来的步骤 3 代码块（`.entry().and_modify().or_insert()`），因为现在已经展开。原来的第 72-75 行成为冗余代码。整个更新块应该是：

```rust
// 3. Lock-free strategy weight update with clamping
let old_weight = self.strategy_weights
    .get(&insight.strategy_id)
    .map(|w| *w);
let strategy_id = insight.strategy_id.clone();
self.strategy_weights
    .entry(strategy_id.clone())
    .and_modify(|w| *w = clamp(*w + delta, -10.0, 10.0))
    .or_insert(clamp(delta, -10.0, 10.0));
let new_weight = self.strategy_weights
    .get(&strategy_id)
    .map(|w| *w)
    .unwrap_or(0.0);
tracing::info!(
    strategy = %strategy_id,
    old = old_weight.unwrap_or(0.0),
    new = new_weight,
    delta,
    lr = lr_t,
    score = insight.score,
    "evolution update"
);
```

- [ ] **Step 2: 记忆存入 JoinHandle 检查 panic**

将第 85-89 行：
```rust
tokio::spawn(async move {
    if let Err(e) = store.upsert(chunk).await {
        tracing::warn!("memory upsert failed: {e}");
    }
});
```
改为：
```rust
let handle = tokio::spawn(async move {
    if let Err(e) = store.upsert(chunk).await {
        tracing::warn!("memory upsert failed: {e}");
    }
});
// Store handle for later panic check (fire-and-forget with panic detection)
// We wrap in a small task that awaits and checks
let check_handle = handle;
tokio::spawn(async move {
    match check_handle.await {
        Ok(()) => {}
        Err(join_err) => {
            tracing::warn!("Memory upsert task panicked: {join_err}");
        }
    }
});
```

- [ ] **Step 3: 文件 I/O 详情使用 debug! 级别**

第 169 行的 `tracing::info!("Evolution state saved to {path}")` 保持 info 级别不变（这是启动/退出时的操作，频率低）。不用改。

第 204-208 行的 `tracing::info!("Loaded evolution state...")` 也保持不变。

- [ ] **Step 4: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add crates/evolution/src/engine.rs
git commit -m "fix(evolution): upgrade weight update to info log, add joinhandle panic detection"
```

---

### Task 5: crates/planner — 策略选择日志、工具序列化错误、重试 warn

**Files:**
- Modify: `crates/planner/src/planner.rs:111, 156, 143-147`

- [ ] **Step 1: best_strategy 返回 None 时 info 日志**

将第 108-111 行：
```rust
let strategy = self
    .evolution
    .best_strategy(&s.candidates.iter().map(|c| c.as_str()).collect::<Vec<_>>())
    .unwrap_or_else(|| "default".into());
```
改为：
```rust
let candidates: Vec<&str> = s.candidates.iter().map(|c| c.as_str()).collect();
let strategy = self
    .evolution
    .best_strategy(&candidates)
    .unwrap_or_else(|| {
        if !candidates.is_empty() {
            tracing::info!(
                tool = %s.tool,
                candidates = ?candidates,
                "no strategy data available, using default"
            );
        }
        "default".into()
    });
```

- [ ] **Step 2: 工具描述序列化失败 error! 日志**

将第 155-156 行：
```rust
serde_json::to_string_pretty(&self.tool_descriptions)
    .unwrap_or_else(|_| "Tools unavailable".into())
```
改为：
```rust
serde_json::to_string_pretty(&self.tool_descriptions)
    .unwrap_or_else(|e| {
        tracing::error!("Failed to serialize tool descriptions: {e}");
        "Tools unavailable".into()
    })
```

- [ ] **Step 3: Planner emit 添加通道断开 warn**

将第 143-147 行 `emit()` 方法：
```rust
fn emit(&self, event: AgentEvent) {
    if let Some(ref tx) = self.event_tx {
        let _ = tx.send(event);
    }
}
```
改为：
```rust
fn emit(&self, event: AgentEvent) {
    if let Some(ref tx) = self.event_tx {
        if tx.send(event).is_err() {
            tracing::warn!("Planner event channel closed");
        }
    }
}
```

- [ ] **Step 4: 重试时打印 warn 日志（CLI 模式也可见）**

在第 71 行的 `PlanRetry` 事件 emit 后添加日志。将第 71 行：
```rust
self.emit(AgentEvent::PlanRetry);
```
改为：
```rust
self.emit(AgentEvent::PlanRetry);
tracing::warn!("Plan parse failed, retrying with clarification prompt");
```

- [ ] **Step 5: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add crates/planner/src/planner.rs
git commit -m "fix(planner): add warn/error logs for strategy fallback, tool serialization, and plan retries"
```

---

### Task 6: crates/scheduler — 错误分类日志、信号量 panic 转 error

**Files:**
- Modify: `crates/scheduler/src/scheduler.rs:97-102, 143-147, 179-183`
- Modify: `crates/scheduler/src/concurrency.rs:24-25`

- [ ] **Step 1: 错误分类日志**

将第 97-102 行：
```rust
Err(e) => agent_core::StepOutput {
    step_id: step.id,
    success: false,
    content: format!("{e}"),
    duration_ms: duration,
},
```
改为：
```rust
Err(e) => {
    let err_msg = format!("{e}");
    let error_category = if err_msg.contains("not found") || err_msg.contains("Tool not found") {
        "tool_not_found"
    } else {
        "tool_error"
    };
    tracing::warn!(
        tool = %step.tool,
        step_id = %step.id,
        category = error_category,
        error = %err_msg,
        "Step execution failed"
    );
    agent_core::StepOutput {
        step_id: step.id,
        success: false,
        content: err_msg,
        duration_ms: duration,
    }
},
```

- [ ] **Step 2: Scheduler emit 添加通道断开 warn**

将第 179-183 行 `emit()` 方法：
```rust
fn emit(&self, event: AgentEvent) {
    if let Some(ref tx) = self.event_tx {
        let _ = tx.send(event);
    }
}
```
改为：
```rust
fn emit(&self, event: AgentEvent) {
    if let Some(ref tx) = self.event_tx {
        if tx.send(event).is_err() {
            tracing::warn!("Scheduler event channel closed");
        }
    }
}
```

- [ ] **Step 3: 信号量关闭改为返回错误而非 panic**

将 `concurrency.rs` 第 20-26 行的 `acquire()` 方法改为：
```rust
pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, anyhow::Error> {
    self.semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| anyhow::anyhow!("Concurrency semaphore closed — agent is shutting down"))
}
```

相应地，在 `scheduler.rs` 中修改 `acquire()` 调用（第 86 行和第 118 行）：

第 86 行：
```rust
let _permit = concurrency.acquire().await;
```
改为：
```rust
let _permit = match concurrency.acquire().await {
    Ok(p) => p,
    Err(e) => {
        return agent_core::StepOutput {
            step_id: step.id,
            success: false,
            content: format!("{e}"),
            duration_ms: 0,
        };
    }
};
```

第 118 行：
```rust
let _permit = concurrency.acquire().await;
```
改为：
```rust
let _permit = match concurrency.acquire().await {
    Ok(p) => p,
    Err(e) => {
        return agent_core::StepOutput {
            step_id: step.id,
            success: false,
            content: format!("retry aborted: {e}"),
            duration_ms: duration,
        };
    }
};
```

- [ ] **Step 4: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 5: 运行测试验证**

```bash
rtk cargo test -p scheduler 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add crates/scheduler/src/scheduler.rs crates/scheduler/src/concurrency.rs
git commit -m "fix(scheduler): add error categorization logs, eliminate semaphore panic"
```

---

### Task 7: crates/tools — web_search 响应失败 fix、bash/file 保留错误类型

**Files:**
- Modify: `crates/tools/src/builtin/web_search.rs:96`

- [ ] **Step 1: web_search 响应体读取失败改为 success: false + warn!**

将第 94-108 行的整个 match 响应体改为：
```rust
match resp {
    Ok(r) if r.status().is_success() => {
        let body = match r.text().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("WebSearch response body read failed: {e}");
                return Ok(ToolOutput::error(format!("Search response read failed: {e}")));
            }
        };
        Ok(ToolOutput {
            success: true,
            content: body,
            metadata: serde_json::json!({"configured": true}),
        })
    }
    Ok(r) => Ok(ToolOutput::error(format!(
        "Search API returned status {}",
        r.status()
    ))),
    Err(e) => Ok(ToolOutput::error(format!("Search request failed: {e}"))),
}
```

- [ ] **Step 2: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/tools/src/builtin/web_search.rs
git commit -m "fix(tools): report web_search response read failure as error instead of silent empty"
```

---

### Task 8: crates/memory — Qdrant 降级原因、无效 UUID warn、嵌入回退 warn

**Files:**
- Modify: `crates/memory/src/vector.rs:46, 152, 160`
- Modify: `crates/memory/src/embedding.rs:41`

- [ ] **Step 1: Qdrant 连接失败时记录具体原因**

将第 45-46 行：
```rust
.map(|r| r.status().is_success())
.unwrap_or(false);
```
改为：
```rust
.map(|r| r.status().is_success())
.unwrap_or_else(|e| {
    tracing::warn!("Qdrant health check failed: {e} — using in-memory fallback");
    false
});
```

同时，将第 51-53 行的 `tracing::warn!(...)` 改为 `tracing::info!(...)`（因为原因已经在上面的 unwrap_or_else 中以 warn 级别记录了）：
```rust
} else {
    tracing::info!(
        "Qdrant not available at {} — using in-memory fallback",
        cfg.url
    );
}
```

- [ ] **Step 2: 无效 UUID 使用 warn! 日志**

将第 152 行：
```rust
id: uuid::Uuid::parse_str(id_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
```
改为：
```rust
id: uuid::Uuid::parse_str(id_str).unwrap_or_else(|e| {
    tracing::warn!("Qdrant returned invalid point UUID {id_str}: {e}, generating new ID");
    uuid::Uuid::new_v4()
}),
```

- [ ] **Step 3: Qdrant 搜索结果为空时打印 debug**

将第 160 行的 `.unwrap_or_default()` 保持，但在返回结果前加日志：
在第 161 行（`Ok(results)`）之前插入：
```rust
if results.is_empty() {
    tracing::debug!("Qdrant search returned empty results");
}
```

- [ ] **Step 4: 嵌入零向量回退改为 warn!**

将 `embedding.rs` 第 41 行：
```rust
tracing::debug!("VoyageEmbedder: no API key, returning zero vector (set VOYAGE_API_KEY env var)");
```
改为：
```rust
tracing::warn!("VoyageEmbedder: no VOYAGE_API_KEY set, using zero vector embeddings (semantic search disabled)");
```

- [ ] **Step 5: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add crates/memory/src/vector.rs crates/memory/src/embedding.rs
git commit -m "fix(memory): add warn logs for Qdrant connection failures, invalid UUIDs, and zero-vector fallback"
```

---

### Task 9: crates/tui — 渲染失败标记错误、JoinHandle panic 检查

**Files:**
- Modify: `crates/tui/src/run.rs:82-83, 255`

- [ ] **Step 1: 渲染失败时记录 error! 日志并标记状态**

将第 80-88 行：
```rust
// 2. Render frame
{
    let state = app_state.read();
    if let Err(e) = terminal.draw(|f| crate::render::render_app(f, &state)) {
        let _ = e;
        break;
    }
    if state.should_quit {
        break;
    }
}
```
改为：
```rust
// 2. Render frame
{
    let state = app_state.read();
    if let Err(e) = terminal.draw(|f| crate::render::render_app(f, &state)) {
        tracing::error!("Terminal draw failed: {e}");
        drop(state);
        let mut state = app_state.write();
        state.should_quit = true;
        state.log_entries.push_back(LogEntry {
            message: format!("Terminal render error: {e}"),
            is_error: true,
        });
        break;
    }
    if state.should_quit {
        break;
    }
}
```

- [ ] **Step 2: TUI JoinHandle panic 检查**

将第 255 行：
```rust
let _ = tui_task.await;
```
改为：
```rust
match tui_task.await {
    Ok(Ok(())) => {}
    Ok(Err(e)) => tracing::error!("TUI render task returned error: {e}"),
    Err(join_err) => tracing::error!("TUI render task panicked: {join_err}"),
}
```

- [ ] **Step 3: 编译验证**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add crates/tui/src/run.rs
git commit -m "fix(tui): add error logging for terminal draw failures and render task panics"
```

---

### Task 10: 最终验证 — 全量编译 + 测试 + 功能验证

- [ ] **Step 1: 全量编译**

```bash
rtk cargo build 2>&1
```

- [ ] **Step 2: 运行全部测试**

```bash
rtk cargo test --workspace 2>&1
```

- [ ] **Step 3: 启动 agent 验证 INFO 日志可见性**

```bash
RUST_LOG=info cargo run -- --config config/default.toml --task "echo hello" 2>&1 | head -50
```

验证输出包含：
- `Hermes agent starting: provider=...`
- `LLM completion request`
- `Turn starting: observe phase`
- `Plan phase`
- `Execute phase`
- `evolution update`
- WARN（如果 API key 未配置）

- [ ] **Step 4: Commit 最终记录**

```bash
git add -A
git commit -m "chore: final verification — all observability optimizations complete"
```

---

## Self-Review Checklist

- [x] 所有 `let _ =` 错误丢弃点都有对应的日志
- [x] 所有降级行为都有 WARN 级别日志
- [x] 所有阶段切换在默认循环中都有 INFO 日志
- [x] 进化权重更新在默认日志级别可见
- [x] 无新增 panic 点（concurrency.rs 的 expect 改为 Result 返回）
- [x] 无 API 签名变更（仅 emit 改为内部实现，不影响调用方；acquire 返回类型变化是唯一 API 变更，通过适配保持兼容）
