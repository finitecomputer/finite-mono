---
title: Orbit API — Primary Documentation Source Note
description: Captured primary product documentation for Orbit API authentication, versioning, and rate-limit retries.
created: 2026-07-25
updated: 2026-07-25
tags:
  - orbit
  - api
  - primary-source
sources:
  - https://docs.orbit.invalid/api
---

# Orbit API — Primary Documentation Source Note

## Provenance

- Publisher: Orbit Project
- Document class: Primary product documentation
- Canonical URL: https://docs.orbit.invalid/api
- Captured from: managed authoritative-document fixture
- Captured on: 2026-07-25

## Captured claims

The primary documentation states that:

- API requests authenticate with a bearer token.
- API requests send `Orbit-Version: 2026-07-01`.
- Clients retry HTTP `429` responses according to the `Retry-After` header.
