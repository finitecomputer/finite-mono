# ADR 0030: Owner Site Redirects

## Decision

A Project's owner may send every request for its Project Site to another
origin, and stop again:

```
POST /api/v2/projects/{slug}/site/redirect     NIP-98, the Project's owner
{"to": "https://garden.finite.place"}         → ProjectSiteRedirectResponse
{"to": null}                                  → serves the Site again
```

`fsite project redirect PROJECT (--to https://HOST | --clear)` wraps it.
Authority is the same as Site visibility: the Project's owner role and
`actor_can_manage_site`. Collaborators edit content; they do not move the Site.

The target is a bare `https` origin: a DNS host name of two or more labels,
with no user info, port, path, query or fragment, and never the service's
own base domain or a Site under it, which would loop or chain Sites
(`finitesites_proto::redirect`, `MAX_REDIRECT_TARGET_BYTES`). It is stored
lowercased, without a trailing slash, in `sites.redirect_to`, with a CHECK as
a backstop. `to` is required, so an empty body never clears a redirect.
Setting the same target again changes nothing and writes no audit row. Each
change writes `redirect_set` or `redirect_cleared` to `site_events` in the
same transaction.

## Serving

A Published Site with a target answers `GET` and `HEAD` with `302 Found`,
`Location: {target}{path}?{query}`, `Cache-Control: no-store` and
`Referrer-Policy: no-referrer`. Those are the same response rules as the
reviewed edge redirects in `infra/scripts/sites-redirects`. The redirect is
checked only after `view_access` allows the visitor, so a private or shared
Site never tells anyone else where it went. A moved Site generates no
`/llms.txt`; that path follows the redirect too. Other methods keep their
`405`. Disabled, unpublished and deleted Sites keep their existing
responses: an operator's disable always wins over an owner's redirect.
`/_finite/*` routes, the API and Git are unaffected. The Project, its
repository and its Versions are kept; Git pushes still publish.

`302` with `no-store`, not `301` or `308`, because the owner can clear the
redirect: a permanent redirect would stay in browsers after it was removed.

## Why

finite.computer agents are moving to finite.chat (FIN-463), where a Site
becomes an app at `*.finite.place`. The old Site's address should keep
working without an operator editing the edge for each one. The owner's
own key decides, through NIP-98 as every other API route does. On finite.chat
that key is the moved agent's Nostr key, held in the person's Vault, which
signs requests to the old hosts without the new agent ever reading it. A
Site's owner could already send visitors anywhere with its own HTML, so an
owner-chosen origin adds no new reach. The server-side redirect makes it
uniform, keeps paths, and works for every file type.

## Considered options

- **Operator-maintained edge mappings** (the existing Caddy redirects):
  reviewed, but one operator edit per Site does not scale to a migration,
  and the owner's own key is the better authority for their Site.
- **Addressing the Site by name** (`/api/v2/sites/{name}/redirect`): the API
  addresses Sites through their Project since ADR 0028, and the old
  platform's export already names each owned Site by its Project Slug.
- **Any URL as the target, path included**: it would make the target join
  ambiguous and widen the validator. An origin keeps every request's own
  path, which is what a moved Site needs.
