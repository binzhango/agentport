# Containerfile
FROM rust:1-bookworm AS builder
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
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