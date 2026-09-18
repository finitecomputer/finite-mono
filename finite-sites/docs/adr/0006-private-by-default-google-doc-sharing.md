# Project Sites Are Private By Default

A new Project Site is `private`. Site visibility and explicit Shares govern
viewer access: `private` admits authorized Native Principals, `shared` also
supports email Shares and viewer sessions, and `public` permits anonymous
reads. Publisher-email authority and account sessions follow
[ADR 0029](0029-account-session-viewer-bridge.md).

Making a Site public requires `confirm_public`, set only after explicit human
confirmation. Site sharing never grants repository edit or Git access. Every
content request checks current permission state, so revocation takes effect
without waiting for a Viewer Cookie to expire.

Sharing applies to the Site, not individual paths. Project collaboration and
repository visibility remain independent controls.
