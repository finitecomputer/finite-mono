# finite-saas-core control-plane image.
# Moved from finitecomputer-v2/deploy/finite-computer/images/core.Dockerfile
# and adapted for the finite-mono root build context (the old COPY paths
# assumed finitecomputer-v2 was the repo root; the crate now builds from the
# root workspace). Build context: repo root. Built ONLY by
# .github/workflows/service-images.yml — never on a prod box.

FROM rust:bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release -p finite-saas-core

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/finite-saas-core /usr/local/bin/finite-saas-core

USER 65532:65532
EXPOSE 4200
CMD ["finite-saas-core", "serve"]
