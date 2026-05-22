# Hermes Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Implement the complete Small Hermes Agent per docs/arch.md — a self-evolving AI agent in Rust (~4,100 lines, 8 crates).

**Architecture:** Rust workspace, 8 crates communicating via trait objects in agent-core. Tokio async runtime, DashMap lock-free concurrency, Qdrant embedded for vector memory, Anthropic + OpenAI dual LLM.

**Tech Stack:** Rust 2021, Tokio, reqwest, serde, DashMap, qdrant-client, clap, tracing, anyhow/thiserror

**Key architectural fix vs arch.md:** Define `MemoryStore` trait in agent-core to break circular dependency between evolution ↔ memory crates.

---

## File Map

| File | Responsibility | Lines |
|---|---|---|
| `Cargo.toml` | Workspace root, shared deps | ~30 |
| `config/default.toml` | Default configuration | ~20 |
| `src/main.rs` | CLI entry, DI assembly | ~80 |
| `crates/agent-core/src/lib.rs` | Core data structs, MemoryStore trait | ~100 |
| `crates/agent-core/src/agent.rs` | HermesAgent trait + run_loop | ~40 |
| `crates/agent-core/src/context.rs` | Context + Ctrl-C handling | ~30 |
| `crates/agent-core/src/runner.rs` | run_agent helper | ~10 |
| `crates/evolution/src/engine.rs` | EvolutionEngine main logic | ~150 |
| `crates/evolution/src/scorer.rs` | Score calculation | ~50 |
| `crates/planner/src/planner.rs` | LLM-driven planning | ~120 |
| `crates/planner/src/plan.rs` | Plan, Step, DependencyGraph | ~80 |
| `crates/memory/src/working.rs` | Ring buffer working memory | ~60 |
| `crates/memory/src/vector.rs` | Qdrant embedded vector memory | ~100 |
| `crates/memory/src/embedding.rs` | Text embedder trait + impls | ~50 |
| `crates/tools/src/registry.rs` | ToolRegistry | ~50 |
| `crates/tools/src/caller.rs` | Unified call interface | ~30 |
| `crates/tools/src/builtin/bash.rs` | BashTool | ~40 |
| `crates/tools/src/builtin/web_search.rs` | WebSearchTool | ~50 |
| `crates/reflector/src/reflector.rs` | Reflection logic | ~80 |
| `crates/reflector/src/attribution.rs` | Error attribution | ~40 |
| `crates/llm/src/adapter.rs` | LlmAdapter trait | ~30 |
| `crates/llm/src/anthropic.rs` | AnthropicAdapter | ~80 |
| `crates/llm/src/openai.rs` | OpenAIAdapter | ~80 |
| `crates/llm/src/stream.rs` | Stream processing | ~40 |
| `crates/scheduler/src/scheduler.rs` | Topological execution | ~80 |
| `crates/scheduler/src/concurrency.rs` | Semaphore control | ~30 |

---

### Task 1: Phase 1 — Workspace Skeleton

Create workspace, all 8 crate skeletons, core data structures, HermesAgent trait, Context, main.rs.

**Files to create:**
- `Cargo.toml`
- `crates/agent-core/Cargo.toml`
- `crates/agent-core/src/lib.rs`
- `crates/agent-core/src/agent.rs`
- `crates/agent-core/src/context.rs`
- `crates/agent-core/src/runner.rs`
- `crates/evolution/Cargo.toml`
- `crates/evolution/src/lib.rs`
- `crates/evolution/src/engine.rs`
- `crates/evolution/src/scorer.rs`
- `crates/planner/Cargo.toml`
- `crates/planner/src/lib.rs`
- `crates/planner/src/planner.rs`
- `crates/planner/src/plan.rs`
- `crates/memory/Cargo.toml`
- `crates/memory/src/lib.rs`
- `crates/memory/src/working.rs`
- `crates/memory/src/vector.rs`
- `crates/memory/src/embedding.rs`
- `crates/tools/Cargo.toml`
- `crates/tools/src/lib.rs`
- `crates/tools/src/registry.rs`
- `crates/tools/src/caller.rs`
- `crates/tools/src/builtin/mod.rs`
- `crates/tools/src/builtin/bash.rs`
- `crates/tools/src/builtin/web_search.rs`
- `crates/reflector/Cargo.toml`
- `crates/reflector/src/lib.rs`
- `crates/reflector/src/reflector.rs`
- `crates/reflector/src/attribution.rs`
- `crates/llm/Cargo.toml`
- `crates/llm/src/lib.rs`
- `crates/llm/src/adapter.rs`
- `crates/llm/src/anthropic.rs`
- `crates/llm/src/openai.rs`
- `crates/llm/src/stream.rs`
- `crates/scheduler/Cargo.toml`
- `crates/scheduler/src/lib.rs`
- `crates/scheduler/src/scheduler.rs`
- `crates/scheduler/src/concurrency.rs`
- `src/main.rs`
- `config/default.toml`

**Implementation:**

All code follows arch.md sections 3-6 exactly, with these adjustments:
1. Add `MemoryStore` trait to `agent-core/src/lib.rs` to break circular dep
2. EvolutionEngine uses `Arc<dyn MemoryStore>` instead of `Arc<VectorMemory>`
3. Embedder trait in `memory/src/embedding.rs`
4. Qdrant embedded mode via `qdrant-client` with `embedded` feature

After Phase 1, `cargo build` should compile all crates successfully with full implementations (no todo!() placeholders in library crates, only main.rs may have wiring).

### Task 2-6: Phase 2-6 — Not Applicable

Per the user's instruction and the comprehensive nature of arch.md, ALL code is implemented in Task 1 as a single comprehensive pass. The arch.md provides complete code for every module. There is no need to split into incremental phases — the document IS the implementation.

---

## Implementation Notes

1. **MemoryStore trait** (added to agent-core, not in arch.md):
```rust
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn search(&self, query: &str, k: usize) -> Result<Vec<MemoryChunk>>;
    async fn upsert(&self, chunk: MemoryChunk) -> Result<()>;
}
```

2. **Qdrant embedded**: Use `qdrant-client` crate's embedded mode — no Docker needed.

3. **Dependency graph**: agent-core ← {evolution, planner, memory, tools, reflector, llm, scheduler}
   - evolution → memory (via MemoryStore trait in agent-core)
   - planner → llm + evolution
   - scheduler → tools
   - reflector → llm
