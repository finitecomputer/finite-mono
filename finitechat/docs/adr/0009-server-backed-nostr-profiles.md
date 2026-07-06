# ADR 0009: Server-Backed Nostr Profiles First

Status: accepted 2026-06-16

Finite Chat uses Nostr account keys as identity and supports Nostr Profile metadata as product UI. In v1, the finitechat server is the first-class profile query/cache backer, and Rust app runtimes keep a local Pika-style profile cache for fast and offline rendering.

Native clients do not query Nostr relays directly in v1. Relay-backed profile lookup can be added later for broader Nostr ecosystem compatibility, but profile data never becomes identity authority: clients still verify account/device membership through Nostr-rooted MLS credentials.
