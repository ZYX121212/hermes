# ── Build stage ──────────────────────────────────────────
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY crates/agent-core/Cargo.toml crates/agent-core/
COPY crates/hermess-agent/Cargo.toml crates/hermess-agent/
COPY crates/evolution/Cargo.toml crates/evolution/
COPY crates/planner/Cargo.toml crates/planner/
COPY crates/memory/Cargo.toml crates/memory/
COPY crates/tools/Cargo.toml crates/tools/
COPY crates/reflector/Cargo.toml crates/reflector/
COPY crates/llm/Cargo.toml crates/llm/
COPY crates/scheduler/Cargo.toml crates/scheduler/
COPY crates/tui/Cargo.toml crates/tui/
COPY crates/hermess-web/Cargo.toml crates/hermess-web/
COPY crates/mcp/Cargo.toml crates/mcp/
COPY crates/hermess-gateway/Cargo.toml crates/hermess-gateway/
COPY src/ src/

# Dummy source files for dependency caching
RUN mkdir -p crates/agent-core/src && echo "" > crates/agent-core/src/lib.rs
RUN mkdir -p crates/hermess-agent/src && echo "" > crates/hermess-agent/src/lib.rs
RUN mkdir -p crates/evolution/src && echo "" > crates/evolution/src/lib.rs
RUN mkdir -p crates/planner/src && echo "" > crates/planner/src/lib.rs
RUN mkdir -p crates/memory/src && echo "" > crates/memory/src/lib.rs
RUN mkdir -p crates/tools/src && echo "" > crates/tools/src/lib.rs
RUN mkdir -p crates/reflector/src && echo "" > crates/reflector/src/lib.rs
RUN mkdir -p crates/llm/src && echo "" > crates/llm/src/lib.rs
RUN mkdir -p crates/scheduler/src && echo "" > crates/scheduler/src/lib.rs
RUN mkdir -p crates/tui/src && echo "" > crates/tui/src/lib.rs
RUN mkdir -p crates/hermess-web/src && echo "" > crates/hermess-web/src/lib.rs
RUN mkdir -p crates/mcp/src && echo "" > crates/mcp/src/lib.rs
RUN mkdir -p crates/hermess-gateway/src && echo "" > crates/hermess-gateway/src/lib.rs

RUN cargo build --release 2>/dev/null || true

# Copy actual source
COPY crates/ crates/

RUN cargo build --release

# ── Runtime stage ────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/hermes /app/hermes
COPY --from=builder /app/target/release/hermess-webd /app/hermess-webd
COPY --from=builder /app/target/release/hermes-gateway /app/hermes-gateway

EXPOSE 8080 8081 9090

ENTRYPOINT ["/app/hermes"]
