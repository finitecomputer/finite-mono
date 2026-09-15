# Experiment result — 2026-09-15

**Verdict: the core architecture works in a live Vercel deployment.** Finite
can own authorization while Vercel serves independently deployed static sites.
The remaining work is product integration and recovery design, rather than a
requirement to operate a dedicated Sites hosting machine.

## What was built

- One native Vercel control API and a real Marketplace Neon database.
- Two independent Vercel site projects, with two content versions for alpha.
- A publisher selecting committed Git files and injecting a platform-owned gate.
- A local interactive console and a real browser sign-in/revocation walkthrough.

Core service code is approximately 200 lines across the control endpoint and
site middleware. The remaining experiment code is publishing, fixtures, probes,
the local demo, and evidence. This is not the full production Sites feature set.

## Proven

- A private site and its assets deny unauthenticated access.
- A trusted synthetic verified-email assertion yields a short-lived one-use
  handoff. Browser redemption sets a Secure, HttpOnly, host-scoped cookie.
- The browser loads the page and its stylesheet after login. After revocation,
  reloading the page and requesting an asset both return 403.
- Current grants apply to every file, including retained deployment URLs after
  authenticating through Vercel's additional Deployment Protection layer.
- The final v2 run passed 55 live HTTP checks across the canonical site URL,
  its retained v1 deployment, and the current v2 deployment.
- Forged identity/middleware headers, HEAD, conditional reads, and encoded asset
  paths do not bypass the tested gate.
- Repeated authorized reads produced Vercel static cache HITs. Revoked reads
  still reached the gate and returned 403; browser responses used `no-store`.
- Alpha's session cannot read beta. Alpha can roll back while beta continues
  serving its own content. Revocation survives rollback and promotion forward.
- Real Postgres tests cover replay, concurrent proof consumption, expiration,
  disabling, grant removal and credential scope. Bad deploy trees fail before
  any remote mutation. All 14 Node tests passed.
- `cargo clippy -p finitesitesd -p fsite-cli --all-targets --locked -- -D warnings`
  passed. Existing Rust services were not changed.

## Findings that affect the design

1. **Retained deployments need current authority.** The content deployment
   contains a site-scoped credential, not a snapshot of permission rules.
2. **Alias convergence is observable.** In the recorded rollback drill, the
   desired content appeared approximately 1.4–3.5 seconds after the CLI operation
   completed. Authorization denial was tested separately without retries.
3. **Browser testing matters.** `Referrer-Policy: no-referrer` caused browser
   form POSTs to carry `Origin: null`. Login/console pages now use `same-origin`
   while retaining strict origin checks. The browser walkthrough passed after
   this fix.
4. **There is request overhead.** Warm canonical-origin probes were generally
   around 100–200 ms from this machine, including the central authorization
   roundtrip. This is a small sample, not a load test or service-level promise.
5. **Recovery is still a product requirement.** This experiment does not make
   Vercel deployments the source of truth for editable Git history or grants.

Raw, credential-free live evidence is under `evidence/`. Deployment identifiers
in those receipts identify the exact tested content/gate versions. The final
browser screenshots are also retained there.

## Boundary

Synthetic viewers and test content only. No production WorkOS bridge, guest
email delivery, native-principal authorization, managed Git repository service,
custom-domain qualification, legacy migration, or complete recovery drill was
performed. See README for the explicit omissions and how to run the experiment.

The canonical `scripts/finite-status` command returned `unknown` before and
after from this laptop because production host evidence is unavailable here.
That is not a green fleet-health assertion. No production Sites, Chat, DNS, or
runtime rollout was performed.
