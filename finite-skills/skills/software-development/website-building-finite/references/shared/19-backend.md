# Backends For Static Sites

Finite Sites serves committed static files. It does not run app processes,
provide a database, or keep mutable server state for a Project Site.

A product needing server secrets, persistence, webhooks, streaming, or server
rendering needs a separately supported backend. Agree on its hosting,
authentication, browser access, and recovery before promising those features.
A local development server is not a deployed backend.

Keep credentials in that backend, outside Git and browser assets. Never embed
service keys in static JavaScript. For inference or media services, follow
`20-llm-api.md` only after establishing the separate backend boundary.

Before delivery, exercise the deployed frontend-to-backend request path,
unauthorized access, and any persistence/recovery promise. Do not describe a
static preview as a working stateful service.
