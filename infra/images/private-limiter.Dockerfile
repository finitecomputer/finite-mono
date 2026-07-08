# finite-private-limiter image (runs inside the Finite Private Tinfoil CVM,
# port 8002). Moved from finitecomputer-v2/deploy/finite-computer/images/ and
# adapted for the finite-mono root build context. Build context: repo root.
# Built ONLY by .github/workflows/service-images.yml.
#
# This image being built HERE closes the old split-brain: the source of truth
# crate is finitecomputer-v2/crates/finite-private-limiter (this repo), while
# the previously-deployed image was built from the legacy finitecomputer repo.
# The digest produced by CI gets pinned in confidential-kimi-k2-6
# (see infra/tinfoil/README.md).

FROM rust:bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release -p finite-private-limiter

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/finite-private-limiter /usr/local/bin/finite-private-limiter

USER 65532:65532
EXPOSE 8002
CMD ["finite-private-limiter"]
