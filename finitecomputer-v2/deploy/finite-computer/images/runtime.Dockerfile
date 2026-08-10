# Build context is the staged finite-mono checkout produced by
# finitecomputer-v2/scripts/build_runtime_image.py. Rust artifacts are built
# together from the one root workspace and lockfile.
#
# The script drives a two-phase build (ADR 0006 payload generations):
#   1. the `finite-payload` stage assembles the payload rootfs at /payload;
#      the script extracts it and packs + signs the seed bundle on the build
#      host (images carry no signing key), writing the result into the build
#      context as seed-payload/;
#   2. the final stage is the shell image: finite-shell as PID 1 plus the
#      signed seed payload at /seed, verified by the shell at first boot
#      against the runtime-provided FINITE_RELEASE_PUBLIC_KEY.

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
COPY finite-release ./finite-release
COPY finite-service-directory ./finite-service-directory
COPY finite-shell ./finite-shell
COPY finitecomputer-v2/crates ./finitecomputer-v2/crates
COPY finitechat ./finitechat
COPY finite-sites ./finite-sites
RUN cargo build --locked --release \
      --package finite-agentd \
      --package finitechat-cli \
      --package fsite-cli \
      --package finite-brain-cli \
      --package finite-shell \
      --package finite-release

# ---------------------------------------------------------------------------
# Payload rootfs: everything the agent iterates on, staged/flipped by
# finite-shell as one generation. Built FROM the same python base as the
# shell image so the Hermes venv's pyvenv.cfg home points at the shell-owned
# interpreter location (/usr/local/bin); the shell's per-generation fixup
# handles the versioned on-disk paths after every unpack.
FROM python:3.13-slim-trixie AS finite-payload
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

# Build-stage-only fetch tooling; the payload ships none of it.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN python -m venv /payload/hermes-venv \
    && /payload/hermes-venv/bin/pip install --no-cache-dir --upgrade pip \
    && test "${HERMES_AGENT_VERSION}" = "0.20.0" \
    && curl -fsSLo /tmp/hermes-agent.tar.gz "${HERMES_AGENT_DIST_URL}" \
    && echo "${HERMES_AGENT_DIST_SHA256}  /tmp/hermes-agent.tar.gz" | sha256sum --check - \
    && HERMES_NIX_BUILD=1 /payload/hermes-venv/bin/pip install --no-cache-dir \
      "hermes-agent[messaging] @ file:///tmp/hermes-agent.tar.gz" \
      "google-api-python-client==2.198.0" \
      "google-auth-oauthlib==1.4.0" \
      "google-auth-httplib2==0.4.0" \
    # The v0.20.0 wheel build drops every non-Python file (package-data only
    # covers hermes_cli schemas + gateway assets), so plugin manifests never
    # install and the plugin manager discovers zero bundled plugins —
    # web_search/web_extract report "no provider configured" even with keys.
    # Overlay the source tree's data files into site-packages so the
    # installed layout matches what their Nix/uv2nix channel ships.
    && mkdir -p /tmp/hermes-dist \
    && tar -xzf /tmp/hermes-agent.tar.gz -C /tmp/hermes-dist --strip-components=1 \
    && cd /tmp/hermes-dist \
    && find agent tools hermes_cli gateway tui_gateway cron acp_adapter plugins providers \
        -type f ! -name "*.py" ! -name "*.pyc" ! -path "*/__pycache__/*" -print0 \
        | tar --null -cf - --files-from - \
        | tar -xf - -C /payload/hermes-venv/lib/python3.13/site-packages/ \
    && cd / \
    && rm -rf /tmp/hermes-dist /tmp/hermes-agent.tar.gz

# Payload bundles admit only relative in-tree symlinks. Replace the venv's
# interpreter entries (absolute symlinks to the base python) with stub files;
# finite-shell's fixup repoints every `python*` bin entry at the shell python
# after each unpack, seed included.
RUN set -eux; \
    for name in /payload/hermes-venv/bin/python*; do \
      rm -f "$name"; \
      printf '%s\n' 'finite-shell fixup repoints this at the shell python' > "$name"; \
    done

COPY --from=finite-rust-builder /build/target/release/finite-agentd /payload/bin/finite-agentd
COPY --from=finite-rust-builder /build/target/release/finitechat /payload/bin/finitechat
COPY --from=finite-rust-builder /build/target/release/fsite /payload/bin/fsite
COPY --from=finite-rust-builder /build/target/release/fbrain /payload/bin/fbrain
COPY finitechat/containers/agent/finite.py /payload/bin/finite

COPY finitechat/integrations/hermes/finitechat /payload/hermes-plugin/finitechat
COPY finite-skills/skills /payload/finite-skills
COPY finitechat/containers/agent/run_hermes_gateway.sh /payload/opt/run_hermes_gateway.sh
COPY finitechat/containers/agent/reconcile_hermes_config.py /payload/opt/reconcile_hermes_config.py
COPY finitechat/containers/agent/recover_chat_boot.py /payload/opt/recover_chat_boot.py
COPY finitechat/containers/agent/probe_hermes_vision.py /payload/opt/probe_hermes_vision.py
COPY finitechat/containers/agent/finite_service_directory.py /payload/opt/finite_service_directory.py

# `bin/` entries become /usr/local/bin shims maintained by the shell, so
# `container exec hermes|finitechat|finite ...` keeps working across
# generations; the relative symlinks keep the Hermes entrypoints reachable
# from agentd's generation-scoped PATH.
RUN chmod +x \
      /payload/bin/finite \
      /payload/opt/run_hermes_gateway.sh \
      /payload/opt/reconcile_hermes_config.py \
      /payload/opt/recover_chat_boot.py \
      /payload/opt/probe_hermes_vision.py \
    && ln -s ../hermes-venv/bin/hermes /payload/bin/hermes \
    && ln -s ../hermes-venv/bin/hermes-agent /payload/bin/hermes-agent \
    && ln -s ../hermes-venv/bin/hermes-acp /payload/bin/hermes-acp

# The pack step runs this stage as a container on the build host, so the
# release tooling must be reachable there without touching /payload.
COPY --from=finite-rust-builder /build/target/release/finite-release /usr/local/bin/finite-release

# ---------------------------------------------------------------------------
# Shell image: the only fixed layer. finite-shell is PID 1; it verifies and
# unpacks the seed payload on first boot, serves /healthz + /contact, and
# supervises the active generation's finite-agentd.
FROM python:3.13-slim-trixie
# The venv (and hermes itself) live in the payload stage now; this ARG only
# feeds the provenance label below.
ARG HERMES_AGENT_VERSION=0.20.0
ARG FINITE_MONO_REV=unknown
ARG GWS_VERSION=0.22.5
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

COPY --from=finite-rust-builder /build/target/release/finite-shell /usr/local/bin/finite-shell

# The signed seed payload: packed and signed on the build host by
# build_runtime_image.py (the image carries no signing key). The shell
# verifies it against the runtime-provided FINITE_RELEASE_PUBLIC_KEY before
# the first unpack; an unverifiable seed never reaches /data.
COPY seed-payload/payload.tar.gz /seed/payload.tar.gz
COPY seed-payload/payload.tar.gz.manifest.json /seed/payload.tar.gz.manifest.json

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
HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 \
  CMD ["curl", "-fsS", "--max-time", "4", "http://127.0.0.1:8080/healthz"]
ENTRYPOINT ["/usr/local/bin/finite-shell", "run"]
