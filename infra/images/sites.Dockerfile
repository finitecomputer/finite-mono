ARG RUST_TOOLCHAIN
FROM rust:${RUST_TOOLCHAIN}-trixie AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# Cargo resolves every workspace member even when building only Sites.
COPY devfinity ./devfinity
COPY finite-agentd ./finite-agentd
COPY finite-brain ./finite-brain
COPY finite-identity ./finite-identity
COPY finite-mail ./finite-mail
COPY finite-nostr ./finite-nostr
COPY finitecomputer-v2/crates ./finitecomputer-v2/crates
COPY finitechat ./finitechat
COPY finite-sites ./finite-sites
RUN cargo build --locked --release -p finitesitesd -p fsite-cli

FROM debian:trixie-slim
ARG FINITE_MONO_REV
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finite-mono" \
      org.opencontainers.image.revision="${FINITE_MONO_REV}"
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 65532 sites \
    && useradd --uid 65532 --gid 65532 --no-create-home sites \
    && mkdir -p /var/lib/finite-sites
COPY --from=builder /src/target/release/finitesitesd /src/target/release/fsite /usr/local/bin/
COPY --chmod=755 infra/images/sites-entrypoint /usr/local/bin/sites-entrypoint
EXPOSE 8787
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/sites-entrypoint"]
CMD ["--help"]
