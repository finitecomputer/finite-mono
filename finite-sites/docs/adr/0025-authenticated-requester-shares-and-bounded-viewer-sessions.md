# Authenticated Requester Shares And Bounded Viewer Sessions

Status: accepted, 2026-07-14.

## Context

An Agent Principal can publish a private Project Output for a human who asked
for it in Finite Chat. The Agent Principal owns the Sites Project, while the
human has a separate User Nostr Identity. Account Auth, possession of a valid
Nostr signature, and ownership of the Agent Runtime do not themselves grant
view access to that Output.

The intended experience is nevertheless direct: the authenticated human who
asked for the publish can open the resulting private Output without proving an
email address or completing a Magic Link flow.

## Decision

- Project Init accepts an optional authenticated `requesting_user_npub`. The
  Agent Principal's signed Project Init is the authorization to create one
  explicit `(Project Output, Native Principal)` Share for that human on every
  declared Output.
- Project creation, Output creation, Native Principal creation, and the
  initial requesting-user Shares occur in one registry transaction. Replay is
  idempotent. The Agent Principal remains the Project owner; the human does not
  become the publisher or receive Git access.
- During an authenticated Finite Chat terminal tool call, Hermes makes its
  task-local session identity available and the Finite Chat adapter leases the
  authenticated sender to `fsite` for only that tool call. `fsite` accepts the
  lease only when the inherited platform, session, and sender all match.
  Standalone agents may still provide `requesting_user_npub` explicitly; if
  it disagrees with an active authenticated lease, Project Init fails closed.
  Agents must not extract or guess identity from message text. Without a
  matching live lease or an explicit value, Project Init creates no implicit
  Share.
- Output owners can add or remove Native Principal Shares explicitly with
  `fsite project share ... --add-npub/--remove-npub`. Email Shares remain
  available for External Principals.
- Sites owns `finite-sites-identity-provider-v1`. Its only hosted operation is
  `authorizeViewerSession`, which signs the exact native-session endpoint and
  bounded challenge body. It is not an arbitrary signing API.
- Electron and iOS may sign that challenge locally with the User Nostr
  Identity and POST directly to the Output host. Hosted Web asks the
  WorkOS-bound Hosted Device to sign the same challenge, then submits it to a
  service-authenticated Sites exchange. The secret key and signed event never
  enter page JavaScript or the Agent Runtime.
- Sites verifies the NIP-98 signature, exact URL, POST method, payload hash,
  freshness, purpose, client, nonce, and same-origin return path. The signer
  must already map to a Native Principal with a Share for that Output. A valid
  proof never creates a Principal or Share.
- Direct exchange sets the normal host-scoped Viewer Cookies. Hosted exchange
  returns a bounded, single-use redemption URL that sets the same cookies.
  Nonces and hosted redemption tokens reject replay.
- Every content request rechecks the Native Principal Share. Removing the
  Share invalidates an otherwise unexpired Viewer Cookie immediately.

Private Outputs may therefore be viewable by explicitly shared Native
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

The existing verified-email viewer-session exchange remains as a compatibility
path for External Principal Shares; it is not used by this native happy path.
