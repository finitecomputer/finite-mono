# Build context = a staged directory containing this repo:
#   ctx/finitechat/

FROM rust:1-trixie AS builder
WORKDIR /build
COPY finitechat ./finitechat
RUN cd finitechat && cargo build --release -p finitechat-cli

FROM python:3.13-slim-trixie
ARG HERMES_AGENT_VERSION=0.20.0
ARG HERMES_AGENT_STORE_PATH
ARG HERMES_AGENT_PYTHON_PATH
ARG AGENT_RUNTIME_TOOLCHAIN_PATH
ARG AGENT_RUNTIME_TOOLCHAIN_BINS
ARG PLAYWRIGHT_BROWSERS_PATH
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates restic \
    && rm -rf /var/lib/apt/lists/*
COPY .finite-hermes-nix-store/nix/store /nix/store
RUN test -n "${HERMES_AGENT_STORE_PATH}" \
    && test -n "${HERMES_AGENT_PYTHON_PATH}" \
    && test -n "${AGENT_RUNTIME_TOOLCHAIN_PATH}" \
    && test -n "${PLAYWRIGHT_BROWSERS_PATH}" \
    && test -x "${HERMES_AGENT_STORE_PATH}/bin/hermes" \
    && test -x "${HERMES_AGENT_PYTHON_PATH}/bin/python3" \
    && test "$("${HERMES_AGENT_PYTHON_PATH}/bin/python3" -c 'import importlib.metadata; print(importlib.metadata.version("hermes-agent"))')" = "${HERMES_AGENT_VERSION}" \
    && for bin in ${AGENT_RUNTIME_TOOLCHAIN_BINS}; do \
         test -x "${AGENT_RUNTIME_TOOLCHAIN_PATH}/bin/${bin}"; \
         ln -sf "${AGENT_RUNTIME_TOOLCHAIN_PATH}/bin/${bin}" "/usr/local/bin/${bin}"; \
       done \
    && test -d "${PLAYWRIGHT_BROWSERS_PATH}" \
    && ln -sf "${HERMES_AGENT_STORE_PATH}/bin/hermes" /usr/local/bin/hermes \
    && ln -sf "${HERMES_AGENT_STORE_PATH}/bin/hermes-agent" /usr/local/bin/hermes-agent \
    && ln -sf "${HERMES_AGENT_STORE_PATH}/bin/hermes-acp" /usr/local/bin/hermes-acp \
    && ln -sf "${HERMES_AGENT_PYTHON_PATH}/bin/python3" /usr/local/bin/python3 \
    && ln -sf "${HERMES_AGENT_PYTHON_PATH}/bin/python3" /usr/local/bin/python
COPY --from=builder /build/finitechat/target/release/finitechat /usr/local/bin/finitechat
COPY finitechat/containers/agent/entrypoint.sh /opt/agent-entrypoint.sh
COPY finitechat/containers/agent/health_server.py /opt/health_server.py
COPY finitechat/containers/agent/reconcile_hermes_config.py /opt/reconcile_hermes_config.py
COPY finitechat/containers/agent/recover_chat_boot.py /opt/recover_chat_boot.py
COPY finitechat/containers/agent/probe_hermes_vision.py /opt/probe_hermes_vision.py
COPY finitechat/containers/agent/run_hermes_gateway.sh /opt/run_hermes_gateway.sh
RUN chmod +x \
      /opt/agent-entrypoint.sh \
      /opt/health_server.py \
      /opt/reconcile_hermes_config.py \
      /opt/recover_chat_boot.py \
      /opt/probe_hermes_vision.py \
      /opt/run_hermes_gateway.sh
ENV FINITECHAT_HOME=/data/agent
# Shared Finite identity root (Finite Identity Contract v1): the account key
# lands at $FINITE_HOME/identity/identity.json on the durable mount, alongside
# the rest of the agent state.
ENV FINITE_HOME=/data/agent
ENV HERMES_HOME=/data/agent/hermes-home
ENV HERMES_AGENT_STORE_PATH="${HERMES_AGENT_STORE_PATH}"
ENV HERMES_AGENT_PYTHON_PATH="${HERMES_AGENT_PYTHON_PATH}"
ENV AGENT_RUNTIME_TOOLCHAIN_PATH="${AGENT_RUNTIME_TOOLCHAIN_PATH}"
ENV AGENT_RUNTIME_TOOLCHAIN_BINS="${AGENT_RUNTIME_TOOLCHAIN_BINS}"
ENV PLAYWRIGHT_BROWSERS_PATH="${PLAYWRIGHT_BROWSERS_PATH}"
ENV PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
ENV PATH="${AGENT_RUNTIME_TOOLCHAIN_PATH}/bin:${HERMES_AGENT_STORE_PATH}/bin:${HERMES_AGENT_PYTHON_PATH}/bin:/usr/local/bin:${PATH}"
EXPOSE 8080
# FINITE_SERVER_URL is provided at `container run` time (the host's vmnet
# gateway address, e.g. http://192.168.64.1:18787).
ENTRYPOINT ["/opt/agent-entrypoint.sh"]
CMD ["/opt/run_hermes_gateway.sh"]
