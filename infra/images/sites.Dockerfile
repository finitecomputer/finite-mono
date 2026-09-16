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
      borgbackup openssh-client rsync sqlite3 python3 supervisor cron \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 65532 sites \
    && useradd --uid 65532 --gid 65532 --no-create-home sites \
    && mkdir -p /var/lib/finite-sites \
    && install -d -m 0700 /var/lib/finitecomputer/backups/rsync-net
COPY --from=builder /src/target/release/finitesitesd /src/target/release/fsite /usr/local/bin/
COPY --chmod=755 infra/images/sites-entrypoint /usr/local/bin/sites-entrypoint
COPY --chmod=755 infra/images/sites-supervisor.py /usr/local/bin/sites-supervisor.py
COPY --chmod=600 infra/images/sites-supervisor.conf /etc/sites-supervisor.conf
COPY --chmod=600 infra/images/sites-backup.cron /etc/cron.d/sites-backup
COPY --chmod=755 infra/scripts/sites-backup /usr/local/bin/sites-backup
COPY --chmod=755 scripts/snapshot-sqlite /usr/local/bin/snapshot-sqlite
COPY --chmod=755 scripts/finite-status /usr/local/bin/finite-status
COPY --chmod=644 scripts/finite_status.py /usr/local/bin/finite_status.py
EXPOSE 8787
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/sites-entrypoint"]
CMD ["--help"]
