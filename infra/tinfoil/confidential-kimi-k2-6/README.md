# Finite Private serving candidates

## Historical-route compatibility bridge

`tinfoil-config.compatibility-bridge.candidate.yml` prepares a CPU-only,
secret-free measured reverse proxy for the historical generated hostname. It
exists so the GPU inference container can adopt the generic `finite-private`
identity without breaking issued Runtime configurations that still read
`kimi-k2-6.finite.containers.tinfoil.dev`.

The bridge is not an inference candidate and must never be deployed while the
current eight-H200 `kimi-k2-6` container still owns the name. During the model
cutover, first replace the GPU workload with the reviewed `finite-private`
release, prove its new route, and only then recreate `kimi-k2-6` from the bridge
release. Retire the bridge only after the stable custom-domain migration has
converged every reader.

## DeepSeek V4 Flash 0731 production candidate

`tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml` preserves the
best configuration measured on an isolated eight-H200 Tinfoil host on
2026-08-07. It uses DP8+EP, FP8 KV cache, a 393,216-token service ceiling, and
the measured scheduler winner of 128 sequences and 2,048 batched tokens.

DeepSeek is the canonical model name. The served `glm-5-2` name remains only as
a mixed-version compatibility alias. The current `kimi-k2-6` directory,
container name, and generated hostname are historical infrastructure identities
and must not be changed as part of the scheduler rollout.

Exact performance, context, protocol, and soak evidence is recorded in
[`2026-08-07-deepseek-v4-eight-h200-optimization.md`](../../../docs/research/2026-08-07-deepseek-v4-eight-h200-optimization.md).
The guarded production procedure is in
[`finite-private-deepseek-production-update.md`](../../runbooks/finite-private-deepseek-production-update.md).

The candidate retains the measured Tinfoil MPK and immutable runtime image
digest. Passing its repository contract does not authorize a satellite
release, Tinfoil relaunch, container replacement, or DNS change.

## Archived GLM candidate (recovery evidence only)

`tinfoil-config.candidate.yml` preserves the older reviewed GLM configuration
for rollback analysis and historical comparison. It is not the next production
candidate and must not be copied to the public
`finitecomputer/confidential-kimi-k2-6` satellite or released. The DeepSeek
candidate above is the only production-update source in this directory.

The candidate follows upstream
[`tinfoilsh/confidential-glm5-2` v0.0.17](https://github.com/tinfoilsh/confidential-glm5-2/releases/tag/v0.0.17),
commit `84b2e80`, for the model-side changes:

- CVM `0.10.8`;
- the v0.0.17 model image digest
  `sha256:0a73ccd09e52d63ef101ac2911e54760b58ca6e0596cadfd219e096d54b1a396`,
  which incorporates the vLLM 0.24 base update;
- `--enable-prompt-tokens-details`; and
- `--max-num-seqs 32` for bounded concurrency/backpressure.

Finite-specific topology is intentionally preserved:

- the public shim still routes to `finite-private-limiter:8002`, not directly
  to vLLM;
- vLLM remains private on `model-net` at port `8001`;
- only the limiter joins `core-api`, whose egress allowlist contains
  `finite.computer`;
- the three sealed secret names and the GLM model/revision/MPK are unchanged;
- the limiter's process healthcheck remains `/live`; deep `/health` and
  `/ready` remain operator rollout gates; and
- the optional limiter watchdog remains disabled.

Upstream v0.0.17 added Tinfoil shim authentication to `/metrics`. That is not
copied: Finite's shim must remain unauthenticated so the limiter can validate
the Finite API key and perform reserve/settle accounting. Making the outer shim
authenticated would replace, not strengthen, that product boundary. Metrics
therefore retain the existing public behavior in this release candidate.

The limiter is pinned to mono image `2026-07-21.1`, digest
`sha256:5d57ecf462fcb105eae2160dd01493efd825532fb61ee286098bdc1b485ec84b`,
from source `cafe85246bce88201c23a46ec7b33c8e28cc25e4`. CI verified the OCI
revision label, and an independent exact-digest pull passed `/live` with the
expected Finite configuration. This archived file is retained only as recovery
evidence. Do not copy it to the satellite, create a release from it, or relaunch
the enclave from it. Delete it once the stable Finite Private route is complete
and the exact GLM rollback evidence is preserved outside this deploy directory.
