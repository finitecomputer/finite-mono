# Public domain routing

This runbook covers the two browser-facing domain changes requested in
September 2026:

- redirect the `finite.vip` apex to `finite.computer` without changing any
  `*.finite.vip` site or the apex NIP-05 identity document;
- replace the generated `enlightened-gem-52.authkit.app` AuthKit hostname with
  a Finite-owned hostname.

These are separate changes. The apex redirect is repository-owned Traefik
configuration on clawland. The AuthKit hostname is WorkOS and DNS control-plane
state; no secret value or generated DNS target belongs in git.

## `finite.vip` apex redirect

`infra/hosts/clawland/finite-identity-nip05-route.yaml` owns both apex routes:

1. priority `10000` keeps the exact `/.well-known/nostr.json` path on the
   canonical Identity public listener;
2. priority `1` permanently redirects every other `finite.vip` request to the
   same path and query at `https://finite.computer`.

The match is exactly the Traefik host rule for `finite.vip`; it cannot capture
`identity.finite.vip` or a user site under `*.finite.vip`.

Before any production mutation, run the platform status command and retain the
output:

```sh
scripts/finite-status
curl -i 'https://finite.vip/.well-known/nostr.json?name=definitely-unknown'
curl -I 'https://finite.vip/domain-routing-canary?source=preflight'
```

The first request must return `200`, CORS `*`, and an empty names object. The
second currently returns the legacy plain `404` and establishes the baseline.

From an authorized clawland Kubernetes context, review the diff and apply the
one manifest:

```sh
kubectl apply -f infra/hosts/clawland/finite-identity-nip05-route.yaml
```

Then prove the protected path before the redirect:

```sh
curl -i 'https://finite.vip/.well-known/nostr.json?name=definitely-unknown'
curl -I 'https://finite.vip/domain-routing-canary?source=acceptance'
curl -L -o /dev/null -sS -w '%{http_code} %{url_effective}\n' \
  'https://finite.vip/domain-routing-canary?source=acceptance'
scripts/finite-status
```

Acceptance is:

- NIP-05 is still `200`, still has `Access-Control-Allow-Origin: *`, and still
  returns the expected empty result for the unknown name;
- the canary receives `301` with
  `Location: https://finite.computer/domain-routing-canary?source=acceptance`;
- following the redirect reaches `finite.computer` (a product `404` for the
  deliberately unknown canary path is acceptable; the effective URL is the
  routing assertion);
- `scripts/finite-status` is green.

Rollback is re-applying the previously reviewed manifest revision. Do not
delete the namespace or selectorless service: the NIP-05 route depends on both.

## AuthKit custom hostname

Use `auth.finite.computer`. `finite.authkit.app` cannot be claimed because
`authkit.app` is owned by WorkOS. The public entry point already exists at
`https://finite.computer/signup`; it creates the hosted signup authorization
request and keeps the callback at `https://finite.computer/callback`.

Follow the official [WorkOS AuthKit domain
guide](https://workos.com/docs/custom-domains/authkit). With the **Production**
environment selected in the WorkOS dashboard:

1. Open **Domains** and choose **Configure AuthKit domain**.
2. Enter `auth.finite.computer`.
3. At the DNS provider for `finite.computer`, add the exact CNAME name and
   target WorkOS displays. Do not guess or copy a staging target. If DNS is
   ever moved to Cloudflare, this record must be DNS-only, not proxied.
4. Wait for WorkOS verification. WorkOS continues retrying verification for up
   to 72 hours.
5. Do not change `NEXT_PUBLIC_WORKOS_REDIRECT_URI`; production already declares
   `https://finite.computer/callback` in
   `infra/nixos/modules/dashboard.nix`.

Acceptance in a private browser window:

1. Open `https://finite.computer/signup` and confirm the hosted form is served
   on `auth.finite.computer`, not the generated `authkit.app` hostname.
2. Complete a new-user signup and confirm the callback returns to
   `https://finite.computer/callback` and then the dashboard.
3. Log out and sign back in, including any configured social provider, so the
   existing-user path is covered too.

Rollback is selecting the prior generated AuthKit domain in WorkOS. Leave the
CNAME in place until rollback is complete and both signup and login pass, then
remove only that exact CNAME.
