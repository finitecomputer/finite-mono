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
ARG HERMES_AGENT_VERSION=0.18.2
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

RUN python -m venv /runtime/hermes-venv \
    && /runtime/hermes-venv/bin/pip install --no-cache-dir --upgrade pip \
    && test "${HERMES_AGENT_VERSION}" = "0.18.2" \
    && /runtime/hermes-venv/bin/pip install --no-cache-dir \
      "hermes-agent[messaging]==${HERMES_AGENT_VERSION}" \
      "google-api-python-client==2.198.0" \
      "google-auth-oauthlib==1.4.0" \
      "google-auth-httplib2==0.4.0" \
    && ln -sf /runtime/hermes-venv/bin/hermes /usr/local/bin/hermes \
    && ln -sf /runtime/hermes-venv/bin/hermes-agent /usr/local/bin/hermes-agent \
    && ln -sf /runtime/hermes-venv/bin/hermes-acp /usr/local/bin/hermes-acp

COPY --from=finite-rust-builder /build/target/release/finitechat /usr/local/bin/finitechat
COPY --from=finite-rust-builder /build/target/release/finitechat /runtime/bin/finitechat
COPY --from=finite-rust-builder /build/target/release/finite-agentd /usr/local/bin/finite-agentd
COPY --from=finite-rust-builder /build/target/release/finite-agentd /runtime/bin/finite-agentd
COPY --from=finite-rust-builder /build/target/release/fsite /usr/local/bin/fsite
COPY --from=finite-rust-builder /build/target/release/fsite /runtime/bin/fsite
COPY --from=finite-rust-builder /build/target/release/fbrain /usr/local/bin/fbrain
COPY --from=finite-rust-builder /build/target/release/fbrain /runtime/bin/fbrain
COPY finitechat/containers/agent/finite.py /runtime/bin/finite

COPY finitechat/integrations/hermes/finitechat /root/.hermes/plugins/finitechat
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
ENV FINITECHAT_HOME=/data/agent
# Shared Finite identity contract: identity.json on the durable mount.
ENV FINITE_HOME=/data/agent
ENV HERMES_HOME=/data/agent/hermes-home
ENV FINITECHAT_WORKSPACE=/data/workspace
ENV FBRAIN_CONFIG_DIR=/data/agent/fbrain
ENV FBRAIN_WORKING_TREE_ROOT=/data/workspace/finitebrain
ENV FINITE_REQUIRE_BUNDLED_SKILLS=1
ENV FINITE_DEFAULT_INFERENCE_PROFILE=finite-private
# The limiter domain keeps the historical kimi-k2-6 name but serves glm-5-2.
ENV FINITE_PRIVATE_BASE_URL=https://kimi-k2-6.finite.containers.tinfoil.dev/v1
ENV FINITE_PRIVATE_MODEL=glm-5-2
ENV FINITECHAT_HERMES_INBOUND_STREAM=1
ENV FINITE_AGENTD_REQUIRED=1

EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 CMD ["/runtime/healthcheck.sh"]
ENTRYPOINT ["/opt/agent-entrypoint.sh"]
CMD ["/runtime/bin/finite-agentd", "serve"]
