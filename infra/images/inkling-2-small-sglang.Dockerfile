# Linux/amd64 manifest reported by Docker Hub for dev-inkling-2-small on
# 2026-11-13. The canary build must still prove pull, provenance and wrapper auth.
FROM lmsysorg/sglang:dev-inkling-2-small@sha256:7cdd32fdd1acdd44207ff2b1483ea776d31769d600241d77fefe878dd0e4fa44

ARG FINITE_MONO_REV
LABEL org.opencontainers.image.source="https://github.com/finitecomputer/finite-mono" \
      org.opencontainers.image.revision="$FINITE_MONO_REV" \
      org.opencontainers.image.base.name="lmsysorg/sglang:dev-inkling-2-small@sha256:7cdd32fdd1acdd44207ff2b1483ea776d31769d600241d77fefe878dd0e4fa44" \
      ai.finite.model="thinkingmachines/Inkling-2-Small@8361e7a31ae7e9966a49a91d3994fde618936c2b"

COPY infra/images/inkling-2-small-sglang-entrypoint.sh /usr/local/bin/finite-inkling-sglang-entrypoint
RUN chmod 0755 /usr/local/bin/finite-inkling-sglang-entrypoint

ENTRYPOINT ["/opt/nvidia/nvidia_entrypoint.sh", "/usr/local/bin/finite-inkling-sglang-entrypoint"]
