# Orbit API

Publisher: Orbit Project
Document class: primary product documentation
Canonical URL: https://docs.orbit.invalid/api

Orbit API requests use the `Orbit-Version: 2026-07-01` header. Clients
authenticate with a bearer token and retry HTTP 429 responses using the
`Retry-After` header.
