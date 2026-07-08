# Multi-stage build for tesserad. Migrations are embedded in the binary via
# sqlx::migrate!, so the runtime image needs only the binary and CA certs.

FROM rust:1.95-slim-bookworm AS builder
WORKDIR /build
# FEATURES lets the cp1 (CPU-only, no Ollama) build enable in-process embeddings:
#   docker build --build-arg FEATURES=fastembed ...
# The default (empty) build uses the Ollama provider.
ARG FEATURES=""
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN if [ -n "$FEATURES" ]; then \
        cargo build --release --bin tesserad --features "tessera-providers/$FEATURES"; \
    else \
        cargo build --release --bin tesserad; \
    fi

FROM debian:bookworm-slim AS runner
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /app tessera
WORKDIR /app
COPY --from=builder /build/target/release/tesserad /usr/local/bin/tesserad
USER tessera
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=5 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["tesserad"]
CMD ["serve"]
