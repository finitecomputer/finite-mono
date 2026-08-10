# Build context is the staged finite-mono checkout produced by
# finitecomputer-v2/scripts/build_runtime_image.py. Rust artifacts are built
# together from the one root workspace and lockfile.

FROM rust:1.88-trixie AS finite-rust-builder
WORKDIR /build
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY devfinity ./devfinity
COPY finite-agentd ./finite-agentd
COPY finite-brain ./finite-brain
COPY finite-identity ./finite-identity
COPY finite-nostr ./finite-nostr
COPY finitecomputer-v2/crates ./finitecomputer-v2/crates
COPY finitechat ./finitechat
COPY finite-sites ./finite-sites
RUN cargo build --locked --release \
      --package finite-agentd \
      --package finitechat-cli \
      --package fsite-cli \
      --package finite-brain-cli

FROM python:3.13-slim-trixie
ARG HERMES_AGENT_VERSION=0.20.0
# Upstream retired the PyPI/brew channels in v0.20.0 (v2026.8.3); supported
# channels are now the shell installer, Docker, and Nix. We install from the
# immutable GitHub source tarball with a pinned sha256 instead. The source
# build guard (setup.py) only permits wheel builds under HERMES_NIX_BUILD=1 —
# we set it explicitly here; NOTE this means we do NOT get their Nix
# derivation's extra hardening (notably the pinned SQLite WAL-reset-fix
# shared library and Node 26 toolchain from their own Dockerfile). Those are
# known gaps tracked on the bump PR, to close before the fleet rollout.
ARG HERMES_AGENT_DIST_URL=https://github.com/NousResearch/hermes-agent/archive/refs/tags/v2026.8.3.tar.gz
ARG HERMES_AGENT_DIST_SHA256=370542c7219faba6300905c3b419e14e6508a31ac698a1a5174e0386990834be
ARG FINITE_MONO_REV=unknown
ARG GWS_VERSION=0.22.5
# Agent toolchains baked into the image so agents stop re-downloading them
# into ephemeral container space at runtime. Every tool is pinned to an exact
# version with sha256 verification.
ARG NODE_VERSION=24.19.0
ARG BUN_VERSION=1.3.14
ARG DENO_VERSION=2.9.5
ARG UV_VERSION=0.12.3
ARG PLAYWRIGHT_VERSION=1.62.0
ARG TARGETARCH

LABEL org.opencontainers.image.title="Finite Computer v2 Agent Runtime"
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finite-mono"
LABEL org.opencontainers.image.revision="${FINITE_MONO_REV}"
LABEL computer.finite.runtime.hermes-agent-version="${HERMES_AGENT_VERSION}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      bash \
      ca-certificates \
      curl \
      git \
      openssh-client \
      restic \
      ripgrep \
      unzip \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) \
        gws_arch=x86_64; \
        gws_sha256=de78ecdbd2f1a84cca0063a7ecbc440240fc14b6ebccbb17f4646b792a8c5c1f \
        ;; \
      arm64) \
        gws_arch=aarch64; \
        gws_sha256=94490295d9580e1e88574e715a0a162991747d12d62f8c7b8dcc8268b6c1cea0 \
        ;; \
      *) echo "unsupported gws architecture: ${TARGETARCH}" >&2; exit 64 ;; \
    esac; \
    archive="google-workspace-cli-${gws_arch}-unknown-linux-gnu.tar.gz"; \
    curl -fsSLo "/tmp/${archive}" \
      "https://github.com/googleworkspace/cli/releases/download/v${GWS_VERSION}/${archive}"; \
    echo "${gws_sha256}  /tmp/${archive}" | sha256sum --check -; \
    tar -xzf "/tmp/${archive}" -C /tmp ./gws; \
    install -m 0755 /tmp/gws /usr/local/bin/gws; \
    rm -f "/tmp/${archive}" /tmp/gws; \
    gws --version

# node (LTS), bun, uv, deno: exact-version archives with sha256
# verification, installed onto PATH so agents find them with no config.
# bun uses the baseline x64 build so it also runs on hosts without AVX2.
# (uv deliberately comes from the GitHub release tarball, not a
# `COPY --from=ghcr.io/astral-sh/uv`: the CI runner's docker cannot
# anonymously pull third-party ghcr images.)
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) \
        node_arch=x64; \
        node_sha256=f625d97cd707df4ff96254916fbc5ff014f09c09effe5a1e0ca8f6d41a8789d4; \
        bun_arch=x64-baseline; \
        bun_sha256=a063908ae08b7852ca10939bbdc6ceed3ddabce8fb9402dce83d65d73b36e6c7; \
        deno_arch=x86_64; \
        deno_sha256=8b010a3b1a4a0188a67cdb8a7a27348b2a501af78aec7fc74f2ace167368d530; \
        uv_arch=x86_64; \
        uv_sha256=600cf9a742aca00d292673b16b5acffaa7b8c269a364ad0c2e79498dcb1fe101; \
        ;; \
      arm64) \
        node_arch=arm64; \
        node_sha256=d28c8a5bf0a808f0ed434a1dce8c54ae98f0371c0bd86ac58abc613f73e6643f; \
        bun_arch=aarch64; \
        bun_sha256=a27ffb63a8310375836e0d6f668ae17fa8d8d18b88c37c821c65331973a19a3b; \
        deno_arch=aarch64; \
        deno_sha256=6b7cae3a8fc4385a59dea3146fcb8bad7fea4230e0ad36a8c692afacbc254be0; \
        uv_arch=aarch64; \
        uv_sha256=bb66cb52e7b1823aed1183630d8d8e5c958840d584a4c55ec10a4cfc168dcca2; \
        ;; \
      *) echo "unsupported toolchain architecture: ${TARGETARCH}" >&2; exit 64 ;; \
    esac; \
    node_archive="node-v${NODE_VERSION}-linux-${node_arch}.tar.gz"; \
    curl -fsSLo "/tmp/${node_archive}" \
      "https://nodejs.org/dist/v${NODE_VERSION}/${node_archive}"; \
    echo "${node_sha256}  /tmp/${node_archive}" | sha256sum --check -; \
    tar -xzf "/tmp/${node_archive}" -C /usr/local --strip-components=1; \
    rm -f "/tmp/${node_archive}"; \
    bun_archive="bun-linux-${bun_arch}.zip"; \
    curl -fsSLo "/tmp/${bun_archive}" \
      "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/${bun_archive}"; \
    echo "${bun_sha256}  /tmp/${bun_archive}" | sha256sum --check -; \
    unzip -q "/tmp/${bun_archive}" -d /tmp; \
    install -m 0755 "/tmp/bun-linux-${bun_arch}/bun" /usr/local/bin/bun; \
    rm -rf "/tmp/${bun_archive}" "/tmp/bun-linux-${bun_arch}"; \
    deno_archive="deno-${deno_arch}-unknown-linux-gnu.zip"; \
    curl -fsSLo "/tmp/${deno_archive}" \
      "https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/${deno_archive}"; \
    echo "${deno_sha256}  /tmp/${deno_archive}" | sha256sum --check -; \
    unzip -q "/tmp/${deno_archive}" -d /tmp/deno-dist; \
    install -m 0755 /tmp/deno-dist/deno /usr/local/bin/deno; \
    rm -rf "/tmp/${deno_archive}" /tmp/deno-dist; \
    uv_archive="uv-${uv_arch}-unknown-linux-gnu.tar.gz"; \
    curl -fsSLo "/tmp/${uv_archive}" \
      "https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/${uv_archive}"; \
    echo "${uv_sha256}  /tmp/${uv_archive}" | sha256sum --check -; \
    tar -xzf "/tmp/${uv_archive}" -C /tmp; \
    install -m 0755 "/tmp/uv-${uv_arch}-unknown-linux-gnu/uv" /usr/local/bin/uv; \
    install -m 0755 "/tmp/uv-${uv_arch}-unknown-linux-gnu/uvx" /usr/local/bin/uvx; \
    rm -rf "/tmp/${uv_archive}" "/tmp/uv-${uv_arch}-unknown-linux-gnu"; \
    node --version; \
    bun --version; \
    uv --version; \
    deno --version

RUN python -m venv /runtime/hermes-venv \
    && /runtime/hermes-venv/bin/pip install --no-cache-dir --upgrade pip \
    && test "${HERMES_AGENT_VERSION}" = "0.20.0" \
    && curl -fsSLo /tmp/hermes-agent.tar.gz "${HERMES_AGENT_DIST_URL}" \
    && echo "${HERMES_AGENT_DIST_SHA256}  /tmp/hermes-agent.tar.gz" | sha256sum --check - \
    && HERMES_NIX_BUILD=1 /runtime/hermes-venv/bin/pip install --no-cache-dir \
      "hermes-agent[messaging] @ file:///tmp/hermes-agent.tar.gz" \
      "google-api-python-client==2.198.0" \
      "google-auth-oauthlib==1.4.0" \
      "google-auth-httplib2==0.4.0" \
      "playwright==${PLAYWRIGHT_VERSION}" \
    && mkdir -p /tmp/hermes-dist \
    && tar -xzf /tmp/hermes-agent.tar.gz -C /tmp/hermes-dist --strip-components=1 \
    && cd /tmp/hermes-dist \
    && find agent tools hermes_cli gateway tui_gateway cron acp_adapter plugins providers \
        -type f ! -name "*.py" ! -name "*.pyc" ! -path "*/__pycache__/*" -print0 \
        | tar --null -cf - --files-from - \
        | tar -xf - -C /runtime/hermes-venv/lib/python3.13/site-packages/ \
    && cd / \
    && rm -rf /tmp/hermes-dist /tmp/hermes-agent.tar.gz \
    && ln -sf /runtime/hermes-venv/bin/hermes /usr/local/bin/hermes \
    && ln -sf /runtime/hermes-venv/bin/hermes-agent /usr/local/bin/hermes-agent \
    && ln -sf /runtime/hermes-venv/bin/hermes-acp /usr/local/bin/hermes-acp

# Pre-install headless chromium (plus its Debian runtime deps) into a shared
# browsers path so agents never download a browser at runtime.
RUN set -eux; \
    /runtime/hermes-venv/bin/playwright install-deps chromium; \
    PLAYWRIGHT_BROWSERS_PATH=/opt/playwright-browsers \
      /runtime/hermes-venv/bin/playwright install chromium; \
    rm -rf /var/lib/apt/lists/*

COPY --from=finite-rust-builder /build/target/release/finitechat /usr/local/bin/finitechat
COPY --from=finite-rust-builder /build/target/release/finitechat /runtime/bin/finitechat
COPY --from=finite-rust-builder /build/target/release/finite-agentd /usr/local/bin/finite-agentd
COPY --from=finite-rust-builder /build/target/release/finite-agentd /runtime/bin/finite-agentd
COPY --from=finite-rust-builder /build/target/release/fsite /usr/local/bin/fsite
COPY --from=finite-rust-builder /build/target/release/fsite /runtime/bin/fsite
COPY --from=finite-rust-builder /build/target/release/fbrain /usr/local/bin/fbrain
COPY --from=finite-rust-builder /build/target/release/fbrain /runtime/bin/fbrain
COPY finitechat/containers/agent/finite.py /runtime/bin/finite

COPY finitechat/integrations/hermes/finitechat /runtime/hermes-plugin/finitechat
COPY finite-skills/skills /runtime/finite-skills
COPY finitechat/containers/agent/entrypoint.sh /opt/agent-entrypoint.sh
COPY finitechat/containers/agent/health_server.py /opt/health_server.py
COPY finitechat/containers/agent/reconcile_hermes_config.py /opt/reconcile_hermes_config.py
COPY finitechat/containers/agent/recover_chat_boot.py /opt/recover_chat_boot.py
COPY finitechat/containers/agent/probe_hermes_vision.py /opt/probe_hermes_vision.py
COPY finitechat/containers/agent/run_hermes_gateway.sh /opt/run_hermes_gateway.sh
COPY finitecomputer-v2/deploy/finite-computer/runtime-template/healthcheck.sh /runtime/healthcheck.sh
COPY finitecomputer-v2/deploy/finite-computer/runtime-template/README.md /runtime/README.md

RUN chmod +x \
      /opt/agent-entrypoint.sh \
      /opt/health_server.py \
      /opt/reconcile_hermes_config.py \
      /opt/recover_chat_boot.py \
      /opt/probe_hermes_vision.py \
      /opt/run_hermes_gateway.sh \
      /runtime/bin/finite \
      /runtime/healthcheck.sh
RUN ln -sf /runtime/bin/finite /usr/local/bin/finite

ENV PATH="/runtime/hermes-venv/bin:/usr/local/bin:${PATH}"
ENV PLAYWRIGHT_BROWSERS_PATH=/opt/playwright-browsers
ENV FINITECHAT_HOME=/data/agent
# Shared Finite identity contract: identity.json on the durable mount.
ENV FINITE_HOME=/data/agent
ENV HERMES_HOME=/data/agent/hermes-home
ENV GOOGLE_WORKSPACE_CLI_CONFIG_DIR=/data/agent/hermes-home/gws
ENV GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE=/data/agent/hermes-home/google_token.json
ENV FINITECHAT_WORKSPACE=/data/workspace
ENV FBRAIN_CONFIG_DIR=/data/agent/fbrain
ENV FBRAIN_WORKING_TREE_ROOT=/data/workspace/finitebrain
ENV FINITE_BRAIN_SERVER_URL=https://brain.finite.computer
ENV FINITE_BRAIN_PUBLIC_BASE_URL=https://brain.finite.computer
ENV FINITE_REQUIRE_BUNDLED_SKILLS=1
ENV FINITE_DEFAULT_INFERENCE_PROFILE=finite-private
# The limiter domain is historical; it now serves DeepSeek V4 Flash 0731.
ENV FINITE_PRIVATE_BASE_URL=https://kimi-k2-6.finite.containers.tinfoil.dev/v1
ENV FINITE_PRIVATE_CONTROL_URL=https://finite.computer/api/core/v1/finite-private
ENV FINITE_PRIVATE_MODEL=deepseek-v4-flash-0731
ENV FINITE_PRIVATE_CONTEXT_LENGTH=393216
ENV FINITECHAT_HERMES_INBOUND_STREAM=1
ENV FINITE_AGENTD_REQUIRED=1

EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 CMD ["/runtime/healthcheck.sh"]
ENTRYPOINT ["/opt/agent-entrypoint.sh"]
CMD ["/runtime/bin/finite-agentd", "serve"]
