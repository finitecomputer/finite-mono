# Eight-H200 Laguna S 2.1 candidate, based on the exact vLLM runtime used in
# the isolated Tinfoil measurements. A published image must be digest-pinned
# in the target candidate before release.
FROM ghcr.io/finitecomputer/deepseek-v4-vllm:0.25.1-0731-reasoning.6@sha256:48716fa9c25605ab5fe00fd7eed4e792268aee6c9008616f7641d9bf622ff262

ARG FINITE_MONO_REV
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finite-mono" \
      org.opencontainers.image.revision="$FINITE_MONO_REV" \
      ai.finite.model="poolside/Laguna-S-2.1-FP8" \
      ai.finite.model.revision="9e0b8ba630080b0e6f20a7b43294a9f2232fd247" \
      ai.finite.draft="poolside/Laguna-S-2.1-DFlash-FP8" \
      ai.finite.draft.revision="a16e2e9287093bf74d7ecd5b5bea732687e0268e"

COPY infra/tinfoil/confidential-kimi-k2-6/laguna-s21-launch.sh /usr/local/bin/
COPY infra/tinfoil/confidential-kimi-k2-6/laguna-s21-router.py /usr/local/bin/
RUN chmod 0755 \
      /usr/local/bin/laguna-s21-launch.sh \
      /usr/local/bin/laguna-s21-router.py

ENTRYPOINT ["/usr/local/bin/laguna-s21-launch.sh"]
