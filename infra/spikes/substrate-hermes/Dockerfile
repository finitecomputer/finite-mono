# Build context contains Nix closures exported from the root flake's exact pins.
FROM debian:13.4
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates bash curl git xvfb openbox xterm x11-utils scrot fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
COPY nix/store /nix/store
ARG HERMES_PATH
ARG PYTHON_PATH
ARG SIMPLEX_PATH
RUN ln -s ${HERMES_PATH}/bin/hermes /usr/local/bin/hermes \
    && ln -s ${PYTHON_PATH}/bin/python3 /usr/local/bin/python3 \
    && ln -s ${SIMPLEX_PATH}/bin/simplex-chat /usr/local/bin/simplex-chat
COPY runtime.py /opt/spike/runtime.py
ENV PYTHONUNBUFFERED=1
WORKDIR /home/agent
ENTRYPOINT ["python3", "/opt/spike/runtime.py"]
