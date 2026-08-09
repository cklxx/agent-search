# syntax=docker/dockerfile:1

# ---- Stage 1: Build agent-search ----
FROM rust:1.80-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY engines.yaml config.toml ./

RUN cargo build --release

# ---- Stage 2: Runtime ----
FROM debian:bookworm-slim

# libssl for reqwest (rustls), ca-certificates for TLS, curl for healthcheck.
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy agent-search binary and configs.
COPY --from=builder /build/target/release/agent-search /usr/local/bin/agent-search
COPY --from=builder /build/engines.yaml /app/engines.yaml
COPY --from=builder /build/config.toml /app/config.toml

WORKDIR /app

EXPOSE 18789

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD curl -fsS http://127.0.0.1:18789/health || exit 1

CMD ["agent-search"]
