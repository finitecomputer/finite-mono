# Finite's H200-stable DeepSeek-V4-Flash-0731 serving candidate.
#
# vLLM 0.25.1 is the tested H200 base used by the public InferenceX recipe.
# The official vLLM 0731 recipe also retreated from 0.26.0 to the 0.25 line
# after a reproducible long-running crash.  The only source modification here
# is upstream vLLM commit 77434861904a9f01ea4818fe9f0c7b2a5c05686e, which
# corrects the 0731 default/high/max reasoning prompt mapping.
FROM vllm/vllm-openai:v0.25.1@sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268 AS rust-builder

# vLLM 0.25.1 serves DeepSeek V4 through the compiled Rust frontend. Build
# only that frontend from the pinned 0.25.1 source after applying the exact
# upstream 0731 patch. The source and patch hashes make both downloads
# fail-closed if either artifact changes.
RUN apt-get update \
  && apt-get install --yes --no-install-recommends \
    build-essential ca-certificates curl libprotobuf-dev libssl-dev patch pkg-config \
    protobuf-compiler \
  && rm -rf /var/lib/apt/lists/* \
  && curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.95 --profile minimal

WORKDIR /build
RUN curl -fsSL \
      https://github.com/vllm-project/vllm/archive/refs/tags/v0.25.1.tar.gz \
      -o vllm-0.25.1.tar.gz \
  && echo "6e41ae186a0623cdf19ce900f47e1be21bec1b7b31646803ac299bda0552e5a0  vllm-0.25.1.tar.gz" \
      | sha256sum --check \
  && curl -fsSL \
      https://github.com/vllm-project/vllm/commit/77434861904a9f01ea4818fe9f0c7b2a5c05686e.patch \
      -o deepseek-v4-0731.patch \
  && echo "65fbd106fd3cad15039cf6a90344ecd993405a4a588d7ebec309a959edcab64d  deepseek-v4-0731.patch" \
      | sha256sum --check \
  && tar -xzf vllm-0.25.1.tar.gz \
  && cd vllm-0.25.1 \
  && patch --forward --batch -p1 < ../deepseek-v4-0731.patch \
  && /root/.cargo/bin/cargo test --locked \
      --manifest-path rust/Cargo.toml \
      -p vllm-chat deepseek_v4 \
  && /root/.cargo/bin/cargo build --locked --release \
      --manifest-path rust/src/cmd/Cargo.toml \
      --bin vllm-rs \
      --features native-tls-vendored

FROM vllm/vllm-openai:v0.25.1@sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268

ARG FINITE_MONO_REV
LABEL org.opencontainers.image.source="https://origin.cursor.com/finite-co/finite-mono" \
      org.opencontainers.image.revision="$FINITE_MONO_REV" \
      org.opencontainers.image.base.name="docker.io/vllm/vllm-openai:v0.25.1" \
      org.opencontainers.image.base.digest="sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268" \
      ai.finite.vllm.upstream-fix="77434861904a9f01ea4818fe9f0c7b2a5c05686e" \
      ai.finite.vllm.rust-frontend-fix="77434861904a9f01ea4818fe9f0c7b2a5c05686e"

COPY infra/images/patch_vllm_deepseek_v4_0731.py /usr/local/libexec/
COPY --from=rust-builder /build/vllm-0.25.1/rust/target/release/vllm-rs /usr/local/lib/python3.12/dist-packages/vllm/vllm-rs
RUN python3 /usr/local/libexec/patch_vllm_deepseek_v4_0731.py apply \
  && python3 /usr/local/libexec/patch_vllm_deepseek_v4_0731.py check \
  && chmod 0755 /usr/local/lib/python3.12/dist-packages/vllm/vllm-rs \
  && python3 -c 'import pathlib, vllm; b=(pathlib.Path(vllm.__path__[0])/"vllm-rs").read_bytes(); assert b"Beyond maximum" in b and b"Absolute maximum" in b'
