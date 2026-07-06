FROM rust:bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p finite-saas-core

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/finite-saas-core /usr/local/bin/finite-saas-core

USER 65532:65532
EXPOSE 4200
CMD ["finite-saas-core", "serve"]
