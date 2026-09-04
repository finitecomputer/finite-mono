# GLM-5.3-Flash degraded admission (temporary)

Torn down 2026-08-29: `v2026-08-28-glm-5-3-flash-5` on
`acc651a6-9de6-4da5-9fdc-bb9888245962` restored usage-api admission. This
file is the historical record of the mid-outage overlay, not current
production.

Temporary limiter mode so the 8xH200 GLM container can serve while
`finite.computer`'s usage-admission route is missing. Not a product feature.
Default remains usage-api; this overlay exists to be torn down.

Live identity:

- Container: `finite-private` (`2aa4d230-0675-4c4a-a7b3-07776b24bfad`)
- Release: `v2026-08-28-glm-5-3-flash-4`
- Overlay: `infra/tinfoil/confidential-finite-private/tinfoil-config.glm-5.3-flash.degraded-allowlist.yml`
- Limiter image: `ghcr.io/finitecomputer/private-limiter:2026-08-28.6@sha256:4746398277eeb7eb96994c40affff34cd070721dd7753596bf7571604c823461`
- SGLang image: unchanged `glm-5-3-flash-sglang:2026-08-28.3`
- DSA: `--dsa-prefill-backend flashmla_sparse --dsa-decode-backend fa3`
- Chunked prefill: `--chunked-prefill-size 16384`
- Context cap: `--context-length 393216` (needle-proven at 387,498 tokens)
- Wire name: `glm-5-3-flash` (hyphenated). Dotted `glm-5.3-flash` is an alias.
- Default thinking: `FINITE_PRIVATE_DEFAULT_REASONING_EFFORT=high` (fill-if-absent)
- Code: PR #746 (allowlist) + PR #748 (DSA pin + thinking default + flash-4)

The GLM checkpoint, SGLang image, and MPK are unchanged from
`v2026-08-28-glm-5-3-flash-1`. `flash-2` was the same overlay with TileLang
DSA and limiter `.5` (no thinking default).

Proved 2026-08-28 22:25 America/Chicago: `/live` and `/health` 200,
`admissionMode=allowlist`, `defaultReasoningEffort=high`,
`defaultEnableThinking=true`, listed-key canary HTTP 200 with
`x-finite-admission: degraded-allowlist` and `model=glm-5-3-flash`,
unlisted key HTTP 401 `invalid_api_key`. Omitted `reasoning_effort` still
returns `reasoning_content`.

## Why it exists

Every inference call goes through the limiter's usage-admission step
(`POST https://finite.computer/internal/finite-private/v1/reservations`).
On 2026-08-28 that path 307'd to the marketing homepage; `/health` on the
same API still answered 200. The GPUs were idle because admission never
reached them, not because GLM or DeepSeek failed. The limiter has no
supported bypass other than this explicit mode.

## What we gave up

- No reservation, no settlement. Tokens served in this mode are unaccounted.
- Only keys in the Tinfoil secret `FINITE_ADMISSION_ALLOWLIST` get through.
  Anyone else is rejected before upstream. The endpoint is not open.
- Settlement-status and accounting gates in the cutover runbook cannot pass
  while this mode is on. Protocol, quality, and capacity still can, because
  they hit the model directly through the limiter.
- `/health` stays green even if the usage API is down (it still reports the
  usage-api probe result; it just does not fail readiness on it).

Observable on every admitted response: `x-finite-admission: degraded-allowlist`.
Health JSON includes `"admissionMode": "allowlist"`.

## What we did not give up

- Auth is still required. An unlisted or missing bearer is 401.
- Model aliases, parsers, context cap, and the SGLang replica are untouched.
- DeepSeek rollback is still one `--replace` onto
  `v2026-08-13-deepseek-v4-flash-0731-128-2048-1`.

## Revert (preferred, once usage admission works)

Recreate from the measured GLM release that uses usage-api. Same host, same
secrets except drop `FINITE_ADMISSION_ALLOWLIST`, no `FINITE_ADMISSION_MODE`:

```
tinfoil container create finite-private \
  --replace $FINITE_PRIVATE_CURRENT_ID \
  --host $FINITE_PRIVATE_HOST \
  --repo finitecomputer/confidential-finite-private \
  --tag v2026-08-28-glm-5-3-flash-1 \
  --secret VLLM_API_KEY \
  --secret VLLM_INTERNAL_API_KEY \
  --secret FINITE_USAGE_API_SERVICE_KEY
```

`flash-1` is the last measured usage-api GLM pin. A later `flash-N` that
keeps the new limiter image but omits `FINITE_ADMISSION_MODE` is the mature
path: the limiter defaults to usage-api, so that image can stay.

Expect another ~35-45 minute checkpoint load. Confirm after ready:

- `/health` JSON has `"admissionMode": "usage-api"` (or the field absent)
- a canary response has no `x-finite-admission` header
- a canary creates a reservation that later settles

## Mature replacement

Do not keep allowlist mode after the product edge is fixed. The missing
route is `POST /internal/finite-private/v1/reservations` on whatever fronts
Core at `finite.computer`. Once that returns a real 2xx/4xx instead of a
homepage redirect:

1. Revert as above.
2. Delete the Tinfoil secret `FINITE_ADMISSION_ALLOWLIST` if nothing else
   references it.
3. Leave the limiter code; the env-gated default is safe to keep. A later
   PR can delete the mode entirely if we decide we never want it again.

## Do not

- Commit the allowlist secret value.
- Ship `FINITE_ADMISSION_MODE=allowlist` as the candidate default.
- Treat a passing capacity sweep in this mode as a passing accounting gate.

## First measurements (2026-08-28 evening, degraded allowlist)

Diagnostic only. Not the 120-user acceptance gate. Evidence:
`.local-state/glm53-cutover-2026-08-28-attempt2/` (not in git).

Thinking off, 64 output tokens (`load-canary`):

| concurrency | per-request p50 tok/s | aggregate tok/s | TTFB p50 |
| --- | --- | --- | --- |
| 1 | 90.5 | 63 | 0.29s |
| 32 | 78.1 | 122 | 15.9s |
| 64 | 69.6 | 237 | 16.3s |

Thinking on, reasoning_effort=high, 256 output tokens (capacity CLI):

| concurrency | per-request p50 tok/s | aggregate tok/s | TTFT p50 / p95 |
| --- | --- | --- | --- |
| 1 | 88.3 | 75 | 0.50s / 0.50s |
| 32 | 56.9 | 218 | 33.1s / 33.1s |

Read against the 120-user hard gate (p50 ≥20 tok/s, p10 ≥10, aggregate ≥2400,
p95 TTFT ≤10s): decode speed is fine at 32-way; time-to-first-token and
aggregate throughput are not. DeepSeek on this same box did ~33 tok/s
per request and ~967 aggregate at 32-way with 0.14s p50 TTFT. GLM's TP8/EP8
replica is faster per request and much slower to start a batch. A 120-way
run is not worth burning until TTFT is in the same zip code as 10s.

## flash-3 DSA swap (2026-08-28 night)

Same overlay, one `--replace` onto `v2026-08-28-glm-5-3-flash-3`. Only serving
deltas vs `flash-2`: DSA `flashmla_sparse`/`fa3` instead of TileLang, and
limiter `.6` filling omitted `reasoning_effort` with `high`. Checkpoint, MPK,
SGLang image, degraded admission, and 8xH200 TP8/EP8 are unchanged.

Thinking off, 64 output tokens (`load-canary`):

| concurrency | flash-2 p50 tok/s | flash-3 p50 | flash-2 TTFB p50 | flash-3 TTFB p50 | flash-3 aggregate |
| --- | --- | --- | --- | --- | --- |
| 1 | 90.5 | 96.9 | 0.29s | 0.68s | 47 |
| 32 | 78.1 | 82.9 | 15.9s | 15.7s | 124 |

Thinking on, reasoning_effort=high, 256 output tokens (capacity CLI):

| concurrency | flash-2 p50 tok/s | flash-3 p50 | flash-2 TTFT p50 | flash-3 TTFT p50 | flash-3 aggregate |
| --- | --- | --- | --- | --- | --- |
| 1 | 88.3 | 95.3 | 0.50s | 0.52s | 80 |
| 32 | 56.9 | 59.4 | 33.1s | 33.8s | 215 |

Decode is a few percent faster. 32-way TTFT did not move. That matches LMSYS's
caveat: the TileLang penalty is large on 24k shared-prefix traffic and inside
noise on short prompts. Our 32-way thinking-on load is the short-prompt case,
so this was the cheapest correct recipe change, not a 120-user fix. Next
one-variable lever is still `--mamba-full-memory-ratio` from boot pool sizes,
then adaptive MTP.

## flash-4 chunked prefill (2026-08-29)

Same overlay, one `--replace` onto `v2026-08-28-glm-5-3-flash-4`. Only serving
delta vs `flash-3`: `--chunked-prefill-size 16384`. Container
`2aa4d230-0675-4c4a-a7b3-07776b24bfad`.

Thinking off, 64 output tokens (`load-canary`):

| concurrency | flash-3 TTFT p50 | flash-4 TTFT p50 | flash-3 aggregate | flash-4 aggregate | flash-4 per-request p50 |
| --- | --- | --- | --- | --- | --- |
| 1 | 0.684s | 0.287s | — | — | 96.8 tok/s |
| 32 | 15.70s | 15.13s | 124.1 | 128.5 | 81.9 tok/s |

The 1-way TTFT win is real. 32-way TTFT only moved ~4%; the queued-burst
edge is still prefill, not decode. 64-way on this box earlier in the
cutover held at 69.6 tok/s p50 per stream / 237.4 aggregate.

393,216 context: a 387,498-token needle through the live limiter retrieved
correctly (cold 21.3s, warm 2.5s). That is one retrieval, not a quality
eval at depth.

Wire name: request `glm-5-3-flash`. The dotted `glm-5.3-flash` 400s unless
listed as an alias.
