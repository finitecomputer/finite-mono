# Build context is staged by scripts/build_runtime_image.py:
#   finitecomputer-v2/
#   finitechat/
#   finite-sites/
#   finite-brain/

FROM rust:1-trixie AS finitechat-builder
WORKDIR /build/finitechat
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY finitechat/Cargo.toml finitechat/Cargo.lock ./
COPY finitechat/crates ./crates
COPY finitechat/integrations ./integrations
COPY finitechat/uniffi-bindgen ./uniffi-bindgen
RUN cargo build --release -p finitechat-cli

FROM rust:1-trixie AS fsite-builder
WORKDIR /build/finite-sites
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY finite-sites/Cargo.toml finite-sites/Cargo.lock ./
COPY finite-sites/crates ./crates
RUN cargo build --release -p fsite-cli

FROM rust:1-trixie AS finite-brain-builder
WORKDIR /build/finite-brain
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY finite-brain/Cargo.toml finite-brain/Cargo.lock ./
COPY finite-brain/crates ./crates
RUN cargo build --release -p finite-brain-cli

FROM python:3.13-slim-trixie
ARG HERMES_AGENT_VERSION=0.18.0
ARG FINITECOMPUTER_V2_REV=unknown
ARG FINITECHAT_REV=unknown
ARG FINITE_SITES_REV=unknown
ARG FINITE_BRAIN_REV=unknown

LABEL org.opencontainers.image.title="Finite Computer v2 Agent Runtime"
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finitecomputer-v2"
LABEL computer.finite.runtime.hermes-agent-version="${HERMES_AGENT_VERSION}"
LABEL computer.finite.source.finitecomputer-v2="${FINITECOMPUTER_V2_REV}"
LABEL computer.finite.source.finitechat="${FINITECHAT_REV}"
LABEL computer.finite.source.finite-sites="${FINITE_SITES_REV}"
LABEL computer.finite.source.finite-brain="${FINITE_BRAIN_REV}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      bash \
      ca-certificates \
      curl \
      git \
      openssh-client \
      restic \
      ripgrep \
    && rm -rf /var/lib/apt/lists/*

RUN python -m venv /runtime/hermes-venv \
    && /runtime/hermes-venv/bin/pip install --no-cache-dir --upgrade pip \
    && /runtime/hermes-venv/bin/pip install --no-cache-dir "hermes-agent==${HERMES_AGENT_VERSION}" \
    && ln -sf /runtime/hermes-venv/bin/hermes /usr/local/bin/hermes \
    && ln -sf /runtime/hermes-venv/bin/hermes-agent /usr/local/bin/hermes-agent \
    && ln -sf /runtime/hermes-venv/bin/hermes-acp /usr/local/bin/hermes-acp

COPY --from=finitechat-builder /build/finitechat/target/release/finitechat /usr/local/bin/finitechat
COPY --from=finitechat-builder /build/finitechat/target/release/finitechat /runtime/bin/finitechat
COPY --from=fsite-builder /build/finite-sites/target/release/fsite /usr/local/bin/fsite
COPY --from=fsite-builder /build/finite-sites/target/release/fsite /runtime/bin/fsite
COPY --from=finite-brain-builder /build/finite-brain/target/release/fbrain /usr/local/bin/fbrain
COPY --from=finite-brain-builder /build/finite-brain/target/release/fbrain /runtime/bin/fbrain

COPY finitechat/integrations/hermes/finitechat /root/.hermes/plugins/finitechat
COPY finitechat/integrations/hermes/finitechat /runtime/hermes-plugin/finitechat
COPY finitechat/containers/agent/entrypoint.sh /opt/agent-entrypoint.sh
COPY finitechat/containers/agent/health_server.py /opt/health_server.py
COPY finitechat/containers/agent/run_hermes_gateway.sh /opt/run_hermes_gateway.sh
COPY finitecomputer-v2/deploy/finite-computer/runtime-template/healthcheck.sh /runtime/healthcheck.sh
COPY finitecomputer-v2/deploy/finite-computer/runtime-template/README.md /runtime/README.md

RUN chmod +x \
      /opt/agent-entrypoint.sh \
      /opt/health_server.py \
      /opt/run_hermes_gateway.sh \
      /runtime/healthcheck.sh

ENV PATH="/runtime/hermes-venv/bin:/usr/local/bin:${PATH}"
ENV FINITECHAT_HOME=/data/agent
# Shared Finite identity contract: identity.json on the durable mount.
ENV FINITE_HOME=/data/agent
ENV HERMES_HOME=/data/agent/hermes-home
ENV FINITECHAT_WORKSPACE=/data/workspace
ENV FINITE_DEFAULT_INFERENCE_PROFILE=finite-private
# The limiter domain keeps the historical kimi-k2-6 name but serves glm-5-2.
ENV FINITE_PRIVATE_BASE_URL=https://kimi-k2-6.finite.containers.tinfoil.dev/v1
ENV FINITE_PRIVATE_MODEL=glm-5-2
ENV FINITECHAT_HERMES_INBOUND_STREAM=1

EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 CMD ["/runtime/healthcheck.sh"]
ENTRYPOINT ["/opt/agent-entrypoint.sh"]
CMD ["/opt/run_hermes_gateway.sh"]
