---
title: Orbit API Client Behavior
summary: Sourced guidance for authenticating, versioning, and retrying Orbit API requests.
created: 2026-07-25
updated: 2026-07-25
tags:
  - orbit
  - api
  - authentication
  - retries
sources:
  - "[[raw/orbit-api.md|Orbit API — Primary Documentation Source Note]]"
---

# Orbit API Client Behavior

Orbit API clients authenticate requests with a bearer token and send the
version header `Orbit-Version: 2026-07-01`.

When the API responds with HTTP `429`, the client should retry according to the
response's `Retry-After` header. The captured documentation does not prescribe
retry behavior for other response codes, so no broader retry policy is asserted
here.

## Source

- [[raw/orbit-api.md|Orbit API — Primary Documentation Source Note]]
