# Hermess Gateway — 多层 LLM 路由网关设计

## 概述

Hermess Gateway 是一个独立的 LLM API 代理网关，对外暴露 OpenAI 兼容的 HTTP API，
内部通过三层架构实现智能路由、成本优化和 token 级质量提升。

## 三层架构

```
Client (OpenAI SDK)
       │ POST /v1/chat/completions
       ▼
Layer 1: axum HTTP Server — OpenAI 兼容格式，单一 API Key 认证
       ▼
Layer 2: Smart Router — SHG 检测 (<1ms) + 分类器 (<50ms) + 策略决策
       ▼
Layer 3: Token Optimizer — Prompt 分解，多模型分发，结果合并 (可选)
       ▼
Backend Adapters — 复用 llm crate 的 OpenAIAdapter / AnthropicAdapter
```

### Layer 1 — 统一接入网关

- 单一 `base_url` + 单一 `api_key`
- 完全兼容 OpenAI SDK 格式
- 客户端无需改动代码，仅改 base_url

### Layer 2 — 智能路由决策层

- SHG (Short-Hard-Guard)：检测"短而难"的请求，匹配 hard_patterns 直接路由到大模型
- 复杂度分类器：调用轻量模型（默认 Qwen-3-Turbo）评估请求复杂度，超时 50ms 则跳过
- 三种路由模式：`cost-first` | `quality-first` | `latency-first`
- 全部开销 <52ms

### Layer 3 — Token 级优化层

- Prompt 分解器：将 prompt 拆为关键部分（→大模型）和常规部分（→小模型），并行调用
- 结果合并器：组装最终输出
- 上下文蒸馏：保留 20% 核心信息（可选，默认关闭）

## 文件结构

新 crate `crates/hermess-gateway/`，不修改任何现有 crate。

```
crates/hermess-gateway/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── server.rs         # axum 路由
│   ├── gateway.rs        # 核心编排
│   ├── config.rs         # 配置结构 + TOML 解析
│   ├── registry.rs       # 模型注册表
│   ├── router/
│   │   ├── mod.rs
│   │   ├── classifier.rs
│   │   ├── shg.rs
│   │   ├── strategy.rs
│   │   └── decision.rs
│   ├── optimizer/
│   │   ├── mod.rs
│   │   ├── decomposer.rs
│   │   ├── merger.rs
│   │   └── distiller.rs
│   └── models.rs
```

## 数据模型

- `RouteMode`: CostFirst | QualityFirst | LatencyFirst
- `ModelCapability`: reasoning, coding, creative, knowledge, speed_ms
- `ModelEntry`: 模型注册信息（含 cost、capability、tags）
- `RouteTarget`: Single(model) | Decomposed { critical, regular }
- `Classification`: complexity, is_short_hard, suggested_tags

## 运行时流程

1. 认证拦截 — 验证 Bearer token
2. SHG 检测 — prompt_len < 200 ∧ 匹配 hard_patterns → 直跳大模型
3. 分类器判定 — 复杂度评分，超时跳过
4. 策略匹配 — 查表选模型
5. 路由决策 — 输出 Single 或 Decomposed
6. 模型调用 — 单模型 或 分解→并行调用→合并
7. 流式透传 — SSE 流式输出，OpenAI 格式

## API 端点

- `POST /v1/chat/completions` — OpenAI 兼容
- `GET /v1/models` — 返回注册模型列表
- `GET /health` — 健康检查
- `POST /v1/embeddings` — 透传 embedding

## 配置

TOML 文件，包含默认模型阵容（Qwen-3-Turbo, DeepSeek-v4, Claude-Opus-4-6），
用户可全量覆盖。支持环境变量插值 (`${VAR_NAME}`)。

## 错误处理

- 分类器超时 → 跳过分类，走默认模型
- 分类器非 JSON → 重试 1 次，失败跳过
- 后端不可达 → 502 OpenAI 格式 error
- 后端超时 → 504
- SHG 大模型挂掉 → 降级 fallback
- decomposer 失败 → 回退 Single

## 测试策略

- 单元测试: SHG, 策略决策, 配置解析, 分解器
- 集成测试: 完整链路 (mock 后端), 流式透传, 认证
- 性能断言: SHG < 1ms, 分类器有超时断路器

## 复用现有 crate

- `llm::OpenAIAdapter` / `llm::AnthropicAdapter` — 调用后端
- `llm::SseChunkStream` — 解析流式响应
- `llm::UsageTracker` — 用量和费用统计
- 不修改任何现有代码
