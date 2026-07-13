# Build context = a staged directory containing this repo:
#   ctx/finitechat/

FROM rust:1-trixie AS builder
WORKDIR /build
COPY finitechat ./finitechat
RUN cd finitechat && cargo build --release -p finitechat-cli

FROM python:3.13-slim-trixie
ARG HERMES_AGENT_VERSION=0.18.2
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates restic \
    && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir "hermes-agent==${HERMES_AGENT_VERSION}"
COPY --from=builder /build/finitechat/target/release/finitechat /usr/local/bin/finitechat
COPY finitechat/integrations/hermes/finitechat /root/.hermes/plugins/finitechat
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
EXPOSE 8080
# FINITE_SERVER_URL is provided at `container run` time (the host's vmnet
# gateway address, e.g. http://192.168.64.1:18787).
ENTRYPOINT ["/opt/agent-entrypoint.sh"]
CMD ["/opt/run_hermes_gateway.sh"]
