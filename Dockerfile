# Containerfile
FROM rust:1-bookworm AS builder
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY README.md ./
COPY src ./src
RUN cargo build --release

FROM node:22-bookworm-slim
ARG CODEX_VERSION=latest
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && npm install --global "@openai/codex@${CODEX_VERSION}" \
    && npm cache clean --force \
    && useradd --create-home tester \
    && mkdir -p /home/tester/.codex /workspace /input \
    && chown -R tester:tester /home/tester /workspace

COPY --from=builder /src/target/release/agentport /usr/local/bin/agentport

USER tester
ENV HOME=/home/tester
ENV CODEX_HOME=/home/tester/.codex
ENV XDG_DATA_HOME=/home/tester/.local/share

WORKDIR /workspace
ENTRYPOINT ["agentport"]
