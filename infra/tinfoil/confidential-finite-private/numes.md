# Numes marketplace serving

The existing serving binary has an opt-in Numes admission path. Tinfoil's shim
still terminates EHBP inside the enclave. A dedicated Numes credential selects
marketplace accounting on `POST /v1/chat/completions`; other inference credentials
retain the existing Finite grant, reservation, settlement and compatibility paths.
Numes traffic never calls Core admission or settlement, including when Core is
unavailable. Numes owns marketplace prices and customer billing.

## Configuration and capacity ownership

No marketplace configuration is enabled in the checked-in serving candidate.
Enabling it requires a reviewed image/configuration, sealed credentials and an
explicitly authorized deployment. Do not give Numes an existing Finite subscriber
key or the model server's internal key.

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

The draft contract requires these headers on every Numes inference request:

- `Authorization: Bearer <dedicated Numes relay credential>`
- `X-Tinfoil-Model: glm-5-3-flash`, exactly matching the decrypted body's model
- `X-Tinfoil-Request-Usage-Metrics: true`

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

Successful non-streaming responses carry `X-Tinfoil-Usage-Metrics` as a header.
Successful streams declare it with `Trailer` and emit it as an HTTP trailer:

```text
prompt=10,completion=2,total=12,model=glm-5-3-flash
```

Counts must be nonnegative exact integers, total must match their sum, completion
must respect the admitted output limit, and the upstream model must match the
listed model. Streaming also requires terminal `[DONE]`. Missing, inconsistent,
malformed or interrupted usage produces **no billable usage header/trailer**;
there are no estimated marketplace charges. Cache discounts and `cost_usd` are
not reported. Numes calculates cost using its approved marketplace rates.

The HTTP response and returned request identifier associate the usage with its
request; this is not a signed receipt, durable usage ledger or retry-deduplication
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

## Qualification before activation

Merging this code does not activate an offer. Publish and review the new serving
image and measured configuration, provision the dedicated credentials, deploy
with authorization, and explicitly raise the live cap from zero. Numes's listing
must reflect the configured output limit and deadline (defaults: 4096 tokens and
120 seconds) and the engine's actual context window. `/v1/models` discovery is
not implemented; supply those facts to Numes explicitly.

The JavaScript SDK/ATC path is a separate activation blocker: this endpoint
reports `tdx-guest/v2`, and ATC rejects its custom-container attestation request
with HTTP 501 because it supports only `sev-snp-guest/v2`. The
[Tinfoil JavaScript verifier](https://docs.tinfoil.sh/sdk/javascript-sdk#what-the-verifier-checks)
also lacks Intel TDX checks. Supporting this deployment requires both a compatible
attestation bundle service and a TDX-capable customer-side verifier, or a separately
qualified deployment on supported hardware. Updating this adapter cannot resolve
that gap. Never bypass verification or substitute relay-side verification for
customer-side verification; prove the approved workload verifies before activation.

Synthetic tests cover local cap races, live pause/raise/lower, client drops during
prefill/streaming, deadlines, valid HTTP/1 streaming trailers, invalid usage and
continued Finite subscription admission/settlement at marketplace saturation.
Still qualify EHBP/trailer transport through the deployed Tinfoil shim and Numes
relay, mismatched encrypted model rejection, actual engine limits/cancellation,
and Finite-user latency under the chosen marketplace cap. Confirm the usage format
and configured listing limits with Numes. Do not enable the offer on the strength
of local tests alone.
