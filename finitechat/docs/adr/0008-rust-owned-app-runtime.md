# ADR 0008: Rust-Owned App Runtime and Pure Native Views

Status: accepted 2026-06-16

Finite Chat's app runtime actor lives in `finitechat-core`. Native UI layers dispatch typed user intents and render Rust-projected `AppState`; they do not own room admission, sync, hint-channel handling, send eligibility, retry policy, or protocol phase transitions. Platform code may own rendering and bounded OS capability bridges, but app policy and durable state stay in Rust so iOS, Android, desktop, CLI, and daemon surfaces share one behavior.

For realtime updates, native code calls the Rust runtime's blocking
`wait_for_update` method from a background task. Rust opens the hint stream,
interprets high-watermarks, pulls authoritative state, admits/finalizes invites,
and returns a new `AppState`. Swift remains a view and action dispatcher; it
does not run polling timers or implement protocol wake logic.
