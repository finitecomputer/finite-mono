# Finite's source-labelled wrapper around SGLang's verified GLM-5.3-Flash
# linux/amd64 image. The upstream tag is intentionally replaced by its exact
# H200-relevant manifest digest.
FROM lmsysorg/sglang:glm-5.3-flash@sha256:0836f0160fa785e424e68d13ef88ddd548f87e6e11ad9f0e4de982e4f9188aaf

ARG FINITE_MONO_REV
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finite-mono" \
      org.opencontainers.image.revision="$FINITE_MONO_REV" \
      org.opencontainers.image.base.name="docker.io/lmsysorg/sglang:glm-5.3-flash" \
      org.opencontainers.image.base.digest="sha256:0836f0160fa785e424e68d13ef88ddd548f87e6e11ad9f0e4de982e4f9188aaf" \
      ai.finite.model="zai-org/GLM-5.3-Flash@04c4e9e95c5da8862dced7e5056455116f83a7e0"

COPY infra/images/glm-5.3-flash-sglang-entrypoint.sh /usr/local/bin/finite-glm53-sglang-entrypoint
RUN chmod 0755 /usr/local/bin/finite-glm53-sglang-entrypoint \
  && test "$(sha256sum /usr/local/bin/finite-glm53-sglang-entrypoint | cut -d ' ' -f 1)" = \
    "da0ee92570967542c309a205c6aba11522777a940e637ff838602a02d73f2b6b"

# Preserve NVIDIA's parent entrypoint so device/library initialization still
# runs. The Finite wrapper then fails closed without the existing sealed
# internal API key and passes it to SGLang without placing its value in the
# public Tinfoil manifest.
ENTRYPOINT ["/opt/nvidia/nvidia_entrypoint.sh", "/usr/local/bin/finite-glm53-sglang-entrypoint"]
