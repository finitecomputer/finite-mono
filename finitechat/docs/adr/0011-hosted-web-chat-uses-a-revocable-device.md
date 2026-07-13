# ADR 0011: Hosted web chat uses a revocable device

Status: accepted 2026-07-09

Finite Computer's launch web-chat surface is a Finite-operated **Hosted Web
Device** with its own server-held device key and durable store, authorized
through Account Auth and presented honestly as trusted web chat rather than
browser E2EE. This preserves the proven dashboard experience without making
Electron or iOS a launch dependency, while Electron and future native clients
enroll as separate revocable Devices with local custody; no device or daemon is
room authority, and server/device restart recovery is a release-gated product
invariant.

This decision does not make WorkOS an account signer. Finite Chat device
credentials are signed by the human's Nostr account key, so launch still needs
an explicit first-login account-key bootstrap and custody contract for Hosted
Web enrollment, later Electron linking, and bring-your-own-nsec behavior. The
device key named above is not that account nsec; until the account-key contract
is accepted, Hosted Web enrollment remains an open launch dependency.

The Hosted Web Device is also part of the SaaS Recovery Set. O1 honestly treats
its Finite-operated process and restored store as potentially accessible during
audited Finite-assisted recovery. Store loss must restore a usable Device and
retained-history/export path; silently minting an unrelated chat account or
showing server ciphertext as recovered data is not acceptable.

ADR 0012 tightens the continuity rule: the Hosted Web Device also owns the
encrypted Project/Principal-to-canonical-Room binding. Navigation selection is
only a cursor, bootstrap opens the binding before Runtime contact, legacy
duplicate exact-member Rooms remain reachable, and recovery cannot mint a
replacement Room.
