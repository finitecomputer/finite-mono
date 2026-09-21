ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ENV PYTHONPATH=/opt/spike
ENV SPIKE_LATENCY_MODE='{"batch_delay":0.05,"reconnect":true,"streaming":true,"wake_commands":["/_app activate"]}'
COPY runtime.py /opt/spike/runtime.py
COPY notifications /opt/spike/notifications
COPY latency/*.py /opt/spike/
COPY patched /tmp/patched
RUN set -e; for root in /nix/store/bs452k1svqd1bbj71p8x66y41za3c1sx-hermes-agent-0.21.0/lib/python3.12/site-packages /nix/store/dlph1rrii0lv8pn671xy460864i9dkfn-hermes-agent-env/lib/python3.12/site-packages; do \
 cp /tmp/patched/adapter.py "$root/plugins/platforms/simplex/adapter.py"; \
 cp /tmp/patched/run.py "$root/gateway/run.py"; \
 cp /tmp/patched/run_agent.py "$root/run_agent.py"; \
 cp /tmp/patched/web_server.py "$root/hermes_cli/web_server.py"; done; \
 cp /tmp/patched/adapter.py /nix/store/mz4g9ydjwn6zd2bb8p2x3rays3nn3s5c-hermes-agent-0.21.0/share/hermes-agent/plugins/platforms/simplex/adapter.py
