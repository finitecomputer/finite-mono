# RFC 0001: Native Finite Sites Viewer Auth

Status: implemented for direct native and Hosted Web adapters, 2026-07-14

## Problem

The native Finite Chat app should open a private or shared Finite Site that has
been shared with the user without sending the user through the email magic-link
flow. This must be invisible in the happy path and must not expose the user's
secret key to the web page, the agent, or any Nostr relay.

## Non-Goals

- Do not replace the current email magic-link flow for external viewers.
- Do not use Nostr relays for this ceremony.
- Do not support remote signers or NIP-46 in Friends Alpha.
- Do not expose signed events or private keys to page JavaScript.
- Do not make site content access depend on cached kind-0 profile metadata.

## Terms

- User Key: the human's native app key. It may sign locally inside the app.
- Native Principal: a Finite Sites Principal known by npub.
- Key Challenge: proof that the actor controls a Nostr key.
- Viewer Cookie: the existing host-scoped cookie used by Finite Sites serving.

## Proposed Flow

When the app opens `https://{site}.finite.chat/{path}` in the in-app browser:

1. The native app recognizes the Finite Sites host.
2. The native app builds a bounded JSON challenge body:

   ```json
   {
     "purpose": "finite_site_view_session",
     "return_to": "/path",
     "client": "finite-chat-ios",
     "nonce": "<client-random>"
   }
   ```

3. The app signs a NIP-98 HTTP auth event locally with the User Key for:

   ```text
   POST https://{site}.finite.chat/_finite/auth/native-session
   ```

   The signature binds the exact URL, method, timestamp, and payload hash.

4. The app submits the request directly to Finite Sites over HTTPS.
5. Finite Sites verifies:

   - the NIP-98 event signature;
   - exact URL and method match;
   - payload hash match;
   - timestamp freshness;
   - request host resolves to the same Finite Site;
   - `purpose == "finite_site_view_session"`;
   - `return_to` is a same-site absolute path, not a URL;
   - signer pubkey maps to a Native Principal;
   - the Native Principal has view access to the Project Output.

6. Finite Sites mints the existing host-scoped HttpOnly Viewer Cookie and
   redirects to `return_to`.
7. The `WKWebView` loads the site normally. Static assets and app-site requests
   use the cookie, not per-request injected auth headers.

Hosted Web uses the same proof contract without exposing custody to the
browser. The dashboard asks the WorkOS-bound Hosted Device's
`finite-sites-identity-provider-v1` adapter to authorize the exact Output
session endpoint. It forwards the returned signed body and Authorization
header to Sites' service-authenticated native viewer exchange. Sites returns a
single-use Output-host redemption URL, which sets the ordinary and Partitioned
Viewer Cookies before the iframe loads.

## Trust Boundary

The user key stays in the native app. The web page sees only ordinary site
traffic after the session cookie exists. The agent never receives the user key
or the signed challenge. Nostr relays are not contacted, published to, or
queried.

Finite Sites verifies the signature locally using the event body; no relay
state is authoritative for access. Access remains a serving-plane decision:
the share table, delegation table, or future entitlement table is re-checked
on every request, so revocation takes effect without waiting for cookie expiry.

An Agent Principal publishing for an authenticated human supplies that human's
npub from trusted Chat sender metadata during Project Init. The Agent-signed
mutation atomically creates an explicit Native Principal Share; it does not
merge the Agent and human identities. Proof exchange can consume an existing
Share but can never create one.

## Endpoint Sketch

```text
POST /_finite/auth/native-session
Host: {site}.finite.chat
Authorization: Nostr <base64-kind-27235-event>
Content-Type: application/json
```

Success:

```text
303 See Other
Set-Cookie: finite_site_auth=<value>; Path=/; Max-Age=...; HttpOnly; SameSite=Lax
Location: <return_to>
```

Failure:

- Return the same unauthorized/login surface used by existing serving-plane
  access checks where possible.
- Do not reveal whether a specific npub has access beyond what a normal
  authenticated viewer would learn.

## Evaluation

Finite Sites tests should cover:

- valid native session;
- stale signature;
- URL mismatch;
- method mismatch;
- payload hash mismatch;
- wrong host;
- unknown site;
- unshared principal;
- revoked share;
- malformed `return_to`;
- oversized body;
- replay behavior if nonce persistence is added.

Finite Chat iOS tests should cover:

- cookie is set before page load;
- private shared site opens without email flow;
- unshared private site does not open;
- public site opens without signing;
- page JavaScript cannot access the key or the signed event;
- relay network disabled does not affect the ceremony.
