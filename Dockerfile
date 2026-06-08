# ── Stage 1: Build ──
FROM rust:1.89-alpine AS builder

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static perl make

WORKDIR /app

# Cache dependencies via dummy src
COPY Cargo.toml Cargo.lock ./
COPY crates/agent-core/Cargo.toml        crates/agent-core/
COPY crates/evolution/Cargo.toml         crates/evolution/
COPY crates/hermess-agent/Cargo.toml     crates/hermess-agent/
COPY crates/hermess-finance/Cargo.toml   crates/hermess-finance/
COPY crates/hermess-gateway/Cargo.toml   crates/hermess-gateway/
COPY crates/hermess-web/Cargo.toml       crates/hermess-web/
COPY crates/llm/Cargo.toml               crates/llm/
COPY crates/mcp/Cargo.toml               crates/mcp/
COPY crates/memory/Cargo.toml            crates/memory/
COPY crates/planner/Cargo.toml           crates/planner/
COPY crates/reflector/Cargo.toml         crates/reflector/
COPY crates/scheduler/Cargo.toml         crates/scheduler/
COPY crates/tools/Cargo.toml             crates/tools/
COPY crates/tui/Cargo.toml               crates/tui/
COPY src/ src/

RUN for d in crates/*/; do mkdir -p "${d}src" && echo "" > "${d}src/lib.rs"; done \
    && cargo build --release 2>/dev/null || true

# Build actual source
COPY crates/ crates/
RUN cargo build --release --bin hermes \
    && strip target/release/hermes

# ── Stage 2: Runtime ──
FROM alpine:3.22

RUN apk add --no-cache \
    ca-certificates bash python3 nodejs \
    chromium chromium-chromedriver curl tzdata \
    && addgroup -S hermes && adduser -S hermes -G hermes

ENV CHROME_BIN=/usr/bin/chromium-browser
ENV TZ=Asia/Shanghai

USER hermes
WORKDIR /home/hermes

COPY --from=builder /app/target/release/hermes /usr/local/bin/hermes
COPY --from=builder /app/config/ ./config/
COPY --from=builder /app/plugins/ ./plugins/

RUN mkdir -p /home/hermes/.hermes

EXPOSE 8080 9090

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["hermes"]
CMD ["--serve", "8080"]
