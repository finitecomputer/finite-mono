# Finite's H200-stable DeepSeek-V4-Flash-0731 serving candidate.
#
# vLLM 0.25.1 is the tested H200 base used by the public InferenceX recipe.
# The official vLLM 0731 recipe also retreated from 0.26.0 to the 0.25 line
# after a reproducible long-running crash.  The only source modification here
# is upstream vLLM commit 77434861904a9f01ea4818fe9f0c7b2a5c05686e, which
# corrects the 0731 default/high/max reasoning prompt mapping.
FROM vllm/vllm-openai:v0.25.1@sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268

ARG FINITE_MONO_REV
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finite-mono" \
      org.opencontainers.image.revision="$FINITE_MONO_REV" \
      org.opencontainers.image.base.name="docker.io/vllm/vllm-openai:v0.25.1" \
      org.opencontainers.image.base.digest="sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268" \
      ai.finite.vllm.upstream-fix="77434861904a9f01ea4818fe9f0c7b2a5c05686e"

COPY infra/images/patch_vllm_deepseek_v4_0731.py /usr/local/libexec/
RUN python3 /usr/local/libexec/patch_vllm_deepseek_v4_0731.py apply \
  && python3 /usr/local/libexec/patch_vllm_deepseek_v4_0731.py check
