# Authenticated Requester Shares And Bounded Viewer Sessions

Status: accepted native-share and native-session contract. Current account
viewer access is defined in [ADR 0029](0029-account-session-viewer-bridge.md).

## Context

An Agent Principal can publish a private Project Site for a human who asked
for it in Finite Chat. The Agent Principal owns the Sites Project, while the
human has a separate User Nostr Identity. Account Auth, possession of a valid
Nostr signature, and ownership of the Agent Runtime do not themselves grant
view access to that Site.

The intended experience is nevertheless direct: the authenticated human who
asked for the publish can open the resulting private Site without proving an
email address or completing a Magic Link flow.

## Decision

- Project Init accepts an optional authenticated `requesting_user_npub`. The
  Agent Principal's signed Project Init is the authorization to create one
  explicit `(Project Site, Native Principal)` Share for that human when the
  Project has a Site.
- Project creation, Site creation, Native Principal creation, and the
  initial requesting-user Shares occur in one registry transaction. Replay is
  idempotent. The Agent Principal remains the Project owner; the human does not
  become the publisher or receive Git access.
- During an authenticated Finite Chat terminal tool call, Hermes makes its
  task-local session identity available and the Finite Chat adapter leases the
  authenticated sender to `fsite` for only that tool call. `fsite` accepts the
  lease only when the inherited platform shape, per-turn session key, sender,
  and adapter-owned authenticated-turn marker all match. The pinned Hermes runtime exposes
  the Finite plugin as `local`; the platform string alone is not the marker.
  Standalone agents may still provide `requesting_user_npub` explicitly; if
  it disagrees with an active authenticated lease, Project Init fails closed.
  Agents must not extract or guess identity from message text. Without a
  matching live lease or an explicit value, Project Init creates no implicit
  Share.
- Site owners can add or remove Native Principal Shares explicitly with
  `fsite project share ... --add-npub/--remove-npub`. Email Shares remain
  available for External Principals.
- Sites owns `finite-sites-identity-provider-v1`. Its only hosted operation is
  `authorizeViewerSession`, which signs the exact native-session endpoint and
  bounded challenge body. It is not an arbitrary signing API.
- Native clients may sign that challenge locally with the User Nostr Identity
  and POST directly to the Site host. The retained Hosted Web native exchange
  accepts the same bounded proof through a service-authenticated endpoint.
  Current dashboard viewing uses the account email exchange in ADR 0029. The
  secret key and signed event never enter page JavaScript or the Agent Runtime.
- Sites verifies the NIP-98 signature, exact URL, POST method, payload hash,
  freshness, purpose, client, nonce, and same-origin return path. The signer
  must already map to a Native Principal with a Share for that Site. A valid
  proof never creates a Principal or Share.
- Direct exchange sets the normal host-scoped Viewer Cookies. Hosted exchange
  returns a bounded, single-use redemption URL that sets the same cookies.
  Nonces and hosted redemption tokens reject replay.
- Every content request rechecks the Native Principal Share. Removing the
  Share invalidates an otherwise unexpired Viewer Cookie immediately.

Private Sites may therefore be viewable by explicitly shared Native
Principals while remaining unavailable to anonymous and email-login viewers.
Changing Visibility to public remains a separate, explicitly confirmed
mutation.

## Consequences

The happy path is: “publish this site” → Agent-signed Project Init with the
authenticated sender identity → private deploy → ordinary site view. There is no
email or Magic Link ceremony for that human.

The design keeps the bounded identity adapter separate from Sites authority:
the adapter proves control of the User Nostr Identity, the Share grants access,
and the Viewer Cookie carries a revocable serving session. Brain grants,
Account Auth, Project collaboration, and Agent ownership are not inherited.

The verified-email viewer-session exchange is the current account dashboard
path. Live qualification of the authenticated requester handoff is tracked in
[FIN-29](https://linear.app/finitecomputer/issue/FIN-29) and
[FIN-53](https://linear.app/finitecomputer/issue/FIN-53); this protocol contract
does not establish that production end-to-end proof.
