# Numes marketplace serving

The existing serving binary has an opt-in Numes admission path. Tinfoil's shim
still terminates EHBP inside the enclave. A dedicated Numes credential selects
marketplace accounting on `POST /v1/chat/completions`; other inference credentials
retain the existing Finite grant, reservation, settlement and compatibility paths.
Numes traffic never calls Core admission or settlement, including when Core is
unavailable. Numes owns marketplace prices and customer billing.

## Configuration and capacity ownership

The checked-in serving candidate pins the marketplace-capable limiter and starts
with capacity zero. It requires sealed `NUMES_API_KEY` and
`NUMES_CONTROL_API_KEY` credentials before deployment. Preparing or publishing
this configuration does not change the running enclave; deployment needs explicit
authorization and admission remains paused until the cap is raised. Do not give
Numes an existing Finite subscriber key or the model server's internal key.

| Variable | Meaning |
| --- | --- |
| `NUMES_API_KEY` | Dedicated relay credential; at least 32 non-space ASCII characters |
| `NUMES_CONTROL_API_KEY` | Separate Finite-only control credential; never give this to Numes |
| `NUMES_MAX_CONCURRENT` | Initial per-process cap; defaults to **0**, paused |
| `NUMES_MAX_OUTPUT_TOKENS` | Per-request completion limit; defaults to 4096, includes reasoning |
| `NUMES_REQUEST_TIMEOUT_SECS` | Whole-request deadline including prefill and delivery; defaults to 120 seconds, at most 3600 |

Keep the initial cap at zero so process replacement always returns to a safe
paused state. Runtime changes are in memory and do not change the measured
configuration or survive a restart. Resume deliberately after checking capacity.
The two credentials must be distinct and must not reuse configured Finite service,
model, metrics or degraded-allowlist credentials.

**Finite's in-enclave cap is authoritative.** Numes can publish its own lower
endpoint/per-model limit, but cannot override ours through request parameters or
headers. Admission atomically checks the total number of Numes requests across
connections, including queueing/prefill, streaming and disconnected-client drains.
There is no waiting queue: a full or paused endpoint immediately returns HTTP 429
with `numes_at_capacity` and `Retry-After: 1`, before contacting the model.

This is a per-process admission limit, intended for the single serving process
in the current enclave. Replicating the service multiplies capacity; a future
multi-replica deployment needs coordinated allocation. A cap bounds request count,
not GPU memory or latency, and provides no preemption or strict scheduling priority
over already-running requests. Qualify the cap against real Finite chat load.

## Tune without deploying

`GET /admin/numes/capacity` reads capacity; `POST` with
`{"max_concurrent": 4}` changes it immediately. Both require the Finite-only
control credential. The accepted range is 0–1024. Lowering the cap never kills
admitted requests: new requests are rejected until `in_flight` is below the cap.
Zero pauses new Numes work while existing requests drain. Finite subscription
requests do not acquire these slots.

Use a protected control-key file outside git:

```sh
export NUMES_CONTROL_KEY_FILE=/path/to/protected/numes-control-key
scripts/with-dev-env python3 infra/runbooks/numes-capacity.py     # inspect
scripts/with-dev-env python3 infra/runbooks/numes-capacity.py 4   # serve up to four
scripts/with-dev-env python3 infra/runbooks/numes-capacity.py 0   # pause and drain
```

The script uses HTTPS and refuses redirects. To verify enclave attestation for
control requests, point `FINITE_PRIVATE_ENDPOINT` at a Tinfoil verification proxy
on `http://127.0.0.1:<port>` pinned to the approved deployment repository/release.
No credential appears in command arguments or script output. Protected `/metrics`
adds `finite_private_numes_max_concurrent` and `finite_private_numes_in_flight`.

## Relay and accounting contract

Numes's Tinfoil adapter requires these headers on every inference request:

- `Authorization: Bearer <dedicated Numes relay credential>`
- `X-Tinfoil-Model: glm-5-3-flash`, exactly matching the decrypted body's model
- `X-Tinfoil-Request-Usage-Metrics: true`

On the public endpoint, the adapter also sends `Content-Type: application/json`,
`Ehbp-Encapsulated-Key` (64 lowercase hexadecimal characters), and `TE: trailers`.
Despite the content type, the public request body is EHBP ciphertext. The shim
decrypts it before this limiter receives JSON. `X-Tinfoil-Enclave-Url` is Numes
routing metadata and is not forwarded to the limiter. Serve the configured URL
directly over TLS 1.3; the relay does not follow redirects.

`X-Request-Id` is optional. When omitted, the adapter generates a correlation ID;
when supplied, it must be a single opaque 1–128 character ASCII identifier using
letters, digits, `-`, `_`, `.`, or `:`. Either ID is returned on the response.

Aliases are reserved for existing Finite clients. Numes lists the canonical
model only. The model server must enforce its published context window. The
adapter enforces one completion and a bounded output limit even when the body
omits it; `X-Tinfoil-Max-Output-Tokens` can further reduce that limit and
`X-Tinfoil-Completion-Count`, if present, must be `1`. Streaming requests always
request engine usage. Reasoning/tool behavior otherwise follows the configured
model and existing thinking defaults.

The client may omit `stream`, `n`, and both optional output/count headers.
`user_cache_secret` remains inside the encrypted body and is forwarded unchanged
to the engine; verify that the deployed engine accepts or safely ignores it.
Its presence does not enable cache billing.

Successful non-streaming responses carry `X-Tinfoil-Usage-Metrics` as a header.
Successful streams declare it with `Trailer` and emit it as an HTTP trailer:

```text
prompt=10,completion=2,total=12,model=glm-5-3-flash
```

Counts must be nonnegative exact integers, total must match their sum, completion
must respect the admitted output limit, and the upstream model must match the
listed model. Streaming requires one final usage event, terminal `[DONE]`, and
normal upstream HTTP EOF. Data after final usage (other than `[DONE]`), repeated
usage, or data after `[DONE]` invalidates the receipt. Missing, inconsistent,
malformed or interrupted usage produces **no billable usage header/trailer**;
there are no estimated marketplace charges. Cache discounts and `cost_usd` are
not reported. Numes calculates cost using its approved marketplace rates.

The same HTTP request/response exchange binds usage to the attempt; Numes does
not require a request identifier in the usage value. This is not a signed
receipt, durable usage ledger or retry-deduplication
API. A retry is a new inference request. Counts remain visible outside EHBP;
request and response content do not.

The generation worker owns its capacity slot before upstream dispatch. If the
HTTP handler or downstream stream is dropped, the worker continues consuming
upstream output to final usage, within the whole-request deadline. It never
releases a slot merely because a customer stopped reading. Slow readers are
bounded by the same deadline. On timeout the adapter closes the upstream request
and emits no fabricated usage; engine-side cancellation latency must be qualified.
For Numes to receive usage after a customer disconnects, its relay must keep its
own upstream response open and drain it. A disconnected relay cannot receive a
trailer; no out-of-band settlement is implemented here.

## Temporary Numes attestation policy

For the initial integration agreed with Numes, its adapter automatically uses
**provider-managed encryption with attestation marked skipped**. There is no
provider CLI/GUI opt-in. EHBP remains enabled, but customers cannot
cryptographically verify that the encryption key belongs to the approved TDX
workload. Numes must label this mode accurately and exclude it from routes that
require verified enclaves. There is no plaintext fallback.

This exception is specific to Numes's initial listing. It does not change Finite
client verification or the operator's deployment/attestation checks in the
[serving runbook](README.md). A fully client-attested listing is a later, separate
qualification: prove that the selected SDK/service verifies the actual deployed
TDX format and binds the EHBP key to the approved workload. Relay verification
alone does not establish customer-side verification.

## Prepare and qualify a release

Merging this code does not activate an offer. Use Numes's
[integration PR](https://git.numes.ai/numes/numes/pulls/1269) and its November 12
integration requirements for joint qualification. Numes owns its endpoint
profile, compatible agent/client builds, prices, policy labels and marketplace
settlement. Its provider agent is needed to prove the complete encrypted path.

1. Record the current instance, measured tag, image digests and credential
   custody using `scripts/finite-status` and the serving runbook. Preserve the
   current configuration and exact rollback tag. Eight-GPU replacement incurs
   downtime; publishing a release and replacing the instance are separate steps.
2. Build the reviewed commit through `service-images.yml` (`private-limiter`).
   Review its image identity and pin the resulting immutable digest in the
   intended measured configuration. Preserve the current model, aliases,
   admission mode, CVM/security settings, networks and existing secret names.
3. Provision fresh sealed `NUMES_API_KEY` and `NUMES_CONTROL_API_KEY` secrets and
   add both names to the **limiter's** secrets list. Add the following limiter
   environment entries. Neither credential belongs to the model container.

   ```yaml
   - NUMES_MAX_CONCURRENT: "0"
   - NUMES_MAX_OUTPUT_TOKENS: "4096"
   - NUMES_REQUEST_TIMEOUT_SECS: "120"
   ```

4. Publish the measured release and complete both release workflow phases.
   Review the exact current-to-target configuration and rollback before an
   authorized deployment. Verify the running tag/measurement, ordinary Finite
   chat, retained aliases and subscription accounting before testing Numes.
5. Keep the Numes offer disabled. Check the control endpoint reports cap zero;
   a correctly authenticated Numes request must get 429 without model dispatch.
   For an authorized qualification window, raise the cap to **1** with the
   control command above and run the acceptance cases below. Start small and
   measure Finite-user latency before selecting the serving cap.
6. Give Numes the deployed commit, limiter digest, measured release, confirmed
   limits and a fresh relay credential through the agreed secure channel. Never
   share the control or internal engine credentials. Activate the listing only
   after both parties have the passing evidence and agreed prices/capabilities.

The initial inventory candidate is `glm-5-3-flash`, text input/output, a combined
393216-token context and 4096-token maximum output including reasoning. Confirm
these against the deployed engine and limiter. The whole-request deadline is
120 seconds with the configuration above; Numes's 11-minute source timeout does
not extend it. Advertise `max_tokens`; reasoning controls and tools require
separate qualification before advertising. `/v1/models` discovery is optional
for this initial listing because Numes has a reviewed static inventory. Prices
are a separate agreement, not facts supplied by this adapter.

| Live acceptance case | Required evidence |
| --- | --- |
| Buffered encrypted chat | HTTP 200, 64-lowercase-hex response nonce, decryptable body, exact final usage header |
| Streamed encrypted chat | Incremental ciphertext/decryption, `[DONE]`, normal EOF and actual final HTTP usage trailer |
| Minimal client request | Success without request ID or optional output/count headers; `user_cache_secret` accepted |
| Model and limits | Canonical model throughout; model mismatch rejected; actual context/output bounds enforced |
| Failure paths | Missing/invalid usage, truncation, upstream error and deadline cannot settle as successful billing |
| Customer disconnect | After ciphertext delivery starts, Numes drains upstream and can collect final usage; no delivery means no charge |
| Pause, saturation and drain | New work gets 429; admitted work retains capacity through EOF/deadline; engine cancels acceptably on timeout |
| Finite continuity and policy | Ordinary Finite chat/accounting remain healthy; Numes is labeled attestation-skipped and excluded from verified routes |

Verify headers and trailers across **engine → limiter → shim → ingress → Numes
agent**. If initial and trailer usage both appear, they must agree exactly. Use
the official EHBP implementation; do not infer encrypted transport success from
plaintext limiter tests or rewrite the shim without a reproduced forwarding
defect. Keep evidence to identities, statuses, usage counts and timings, without
credentials, prompts, cache secrets or generated content.

If qualification fails or Finite latency degrades, set the live cap to zero and
disable the Numes offer. Observe `in_flight` draining before a replacement. A
restart returns to the configured zero cap. Follow the serving runbook for an
authorized rollback to the preserved measured release; do not use the outdated
degraded candidate as an automatic fallback.

Synthetic tests cover cap races, pause/raise/lower, client drops during
prefill/streaming, deadlines, HTTP/1 trailers, invalid/nonfinal usage and existing
Finite subscription admission/settlement at saturation. They do not qualify
the live shim, engine, Numes agent or marketplace settlement.
