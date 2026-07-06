FROM rust:bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p finite-private-limiter

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/finite-private-limiter /usr/local/bin/finite-private-limiter

USER 65532:65532
EXPOSE 8002
CMD ["finite-private-limiter"]
