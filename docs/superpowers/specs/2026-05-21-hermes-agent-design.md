# Hermes Agent Design Spec

**Date**: 2026-05-21
**Status**: Approved
**Based on**: docs/arch.md v1.0

## Overview

Hermes is a minimal self-evolving AI Agent implemented in Rust (~4,100 lines). It follows a five-step feedback loop: Observe → Plan → Execute → Reflect → Evolve.

## Technical Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Project name | `hermes` | User confirmed |
| LLM adapters | Anthropic + OpenAI dual implementation | User confirmed |
| Vector storage | Qdrant embedded mode | No external process needed |
| Concurrency | Tokio async/await | IO-intensive tool calls |
| Weight storage | DashMap (lock-free) | Read-heavy, low contention |
| Learning rate | AtomicU64 storing f64 bits | No Mutex needed |
| Tool registration | Arc<dyn Tool> trait objects | Runtime dynamic registration |
| Error handling | anyhow + thiserror | anyhow for prototyping, thiserror at lib boundaries |
| Config format | TOML | serde native, Rust ecosystem standard |
| DAG execution | Topological sort by layer | Max concurrency within each layer |

## Architecture

### Workspace Structure (8 crates)

```
hermes/
├── Cargo.toml               # workspace root
├── config/
│   ├── default.toml
│   └── production.toml
├── crates/
│   ├── agent-core/          # Main loop, lifecycle, Context (~420 lines)
│   ├── evolution/           # Evolution engine (~680 lines)
│   ├── planner/             # Task planning (~350 lines)
│   ├── memory/              # Working + vector memory (~480 lines)
│   ├── tools/               # Tool registry + builtins (~310 lines)
│   ├── reflector/           # Reflection & scoring (~290 lines)
│   ├── llm/                 # LLM adapters (~380 lines)
│   └── scheduler/           # Async execution (~260 lines)
├── src/
│   └── main.rs              # CLI entry (~80 lines)
└── tests/
    ├── integration/
    └── fixtures/
```

### Five-Step Loop

```
Observe → Plan → Execute → Reflect → Evolve
   ↑________________________________________|
```

1. **Observe**: Collect user input + environment state
2. **Plan**: Decompose goal into DAG of tool-calling steps via LLM
3. **Execute**: Run steps concurrently by topological layer
4. **Reflect**: Score results, attribute errors, generate Insight
5. **Evolve**: Update strategy weights (lock-free), write to long-term memory

### Core Data Structures

- **Observation**: user_input + env_state + memory_ctx
- **Plan**: steps + DAG (DependencyGraph)
- **Step**: tool name + args + dependencies + strategy tag
- **ExecutionResult**: outputs + success flag + duration
- **Insight**: strategy_id + score [-1.0, 1.0] + embedding + lesson

### Key Traits

- `HermesAgent`: observe, plan, execute, reflect, evolve + default run_loop
- `LlmAdapter`: complete, complete_stream, embed
- `Tool`: name, description, schema, call
- `Embedder`: embed(text) -> Vec<f32>

## Implementation Phases

### Phase 1 — Skeleton
- Create workspace, Cargo.toml, dependencies
- Define all core data structures
- Define HermesAgent trait with default run_loop
- Implement minimal Context (with should_stop flag)
- Write main.rs framework (compiles with todo!())

### Phase 2 — Tool Layer
- Tool trait + ToolRegistry
- BashTool + WebSearchTool builtins
- Scheduler: topological sort + concurrent execution

### Phase 3 — LLM Layer
- LlmAdapter trait
- AnthropicAdapter (non-streaming first)
- OpenAIAdapter (non-streaming first)
- Embedder

### Phase 4 — Planning & Memory
- WorkingMemory (ring buffer)
- VectorMemory (Qdrant embedded)
- Planner (LLM-driven task decomposition)

### Phase 5 — Evolution Loop
- Scorer (pure function)
- Reflector (LLM attribution on failure)
- EvolutionEngine (weight update + memory write)
- End-to-end integration: full five-step cycle

### Phase 6 — Polish
- tracing instrumentation
- Adaptive learning rate decay
- TOML config file support
- Concurrent safety stress tests

## Testing Strategy

- **Unit**: Scorer, DependencyGraph, WorkingMemory, EvolutionEngine (mock VectorMemory)
- **Integration**: Tool calls (mock HTTP), LLM calls (recorded fixtures), full cycle (mock LLM with fixed seed)
- **Concurrency**: Concurrent EvolutionEngine updates, verifying weight consistency
