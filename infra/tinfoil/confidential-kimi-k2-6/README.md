# Finite Private next measured config

## DeepSeek-V4-Flash-0731 preparation

The first 2026-08-04 attempt is preserved on its original mono and satellite
branches. DeepSeek was measured and exercised in production, then production
was deliberately rolled back to GLM 5.2. Keep those branches and release tags
as evidence; do not merge the DeepSeek model/runtime-default changes while GLM
remains the production authority.

The target-only `tinfoil-config.deepseek-v4-flash-0731-dspark-off.candidate.yml`
now stages retry 2 as described in
[`finite-private-deepseek-v4-flash-0731-retry-2.md`](../../runbooks/finite-private-deepseek-v4-flash-0731-retry-2.md):

- official model revision
  `deepseek-ai/DeepSeek-V4-Flash-0731@7872f01b1d1fe23eabc4c98b48bffcef5a386062`;
- a Finite compatibility image built from official vLLM 0.25.1 linux/amd64
  digest `sha256:f0b9a0dc75a9fca3b6811e3279367b2d6a448055a000bfd13859587d74cef268`
  with only upstream 0731 reasoning fix `7743486`, guarded by exact source
  hashes;
- eight-H200 data + expert parallelism, FP8 KV cache, 393,216-token service
  ceiling, DeepSeek V4 parsers, and both the canonical model name and retained
  `glm-5-2` request alias;
- target-only generation with DSpark excluded from retry 2; and
- the current production limiter digest and public limiter/shim topology,
  keeping this a model-only change.

The retained `tinfoil-config.deepseek-v4-flash-0731-dspark-on.candidate.yml` is
the first-attempt diagnostic artifact. It is intentionally excluded from the
retry checker and must not be released for retry 2.

The retry file retains the already measured Tinfoil MPK and deliberately
contains `REPLACE_WITH_MEASURED_DEEPSEEK_V4_VLLM_IMAGE`. The prep contract
accepts that explicit placeholder; the release-ready gate fails until the
manual image workflow reports and pins an immutable digest:

```bash
just finite-private-deepseek-contract
just finite-private-deepseek-release-contract
```

Do not publish the image, copy the retry file to the satellite root, create a
release, or relaunch the enclave merely because the prep contract passes.

## Existing GLM/limiter candidate

`tinfoil-config.candidate.yml` is the reviewed source for the next update to
the public `finitecomputer/confidential-kimi-k2-6` satellite. It is staged in
mono so the product, limiter, and enclave changes can be reviewed together.
Tinfoil still requires the released `tinfoil-config.yml` at the satellite repo
root.

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
expected Finite configuration. Do not copy the candidate to the satellite,
create a measured release, or relaunch the enclave without explicit approval
for the downtime operation.
