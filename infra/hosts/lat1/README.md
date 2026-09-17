# finite-lat-1 — retired

lat1 (`64.34.82.77`) is retired. Do not deploy services, enable its writers, or
point production DNS at it. Retained disk state is evidence, not a live fallback.

Current app-plane configuration is [finite-lat-2](../../nixos/hosts/finite-lat-2/).
Use [break-glass](../../runbooks/break-glass.md) for authorized diagnosis and
[recovery](../../runbooks/hosted-web-chat-recovery.md) for restore requirements.

Files here include retained configuration and credential-name examples consumed
by current tooling. Their location does not make lat1 an active host.
