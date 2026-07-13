# Shared AEON specialization worker. Build context: finite-mono root.
# Built only by .github/workflows/service-images.yml.

FROM rust:bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release -p finite-specialization-worker

FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates ffmpeg \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/finite-specialization-worker /usr/local/bin/finite-specialization-worker

USER 65532:65532
EXPOSE 18998
CMD ["finite-specialization-worker", "serve", "--host", "0.0.0.0", "--port", "18998"]
