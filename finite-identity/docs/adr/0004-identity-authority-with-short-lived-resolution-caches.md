# Identity Authority with short-lived Resolution Caches

Finite Identity is the Identity Authority for Principal Resolution, Finite VIP Email bindings, and NIP-05 Names. Products may keep short-lived Resolution Caches for latency and availability, but permission-changing operations should consult the Identity Authority directly and uncertain authorization must fail closed.
