# ADR 0007: Realtime Hints Behind One Hint Channel

Status: accepted 2026-06-16

Finite Chat app runtimes use a single hint-channel abstraction for realtime wakeups. SSE is the primary app/runtime transport; `/sync/wait` remains as a long-poll fallback for CLI, tests, dev loops, and environments where SSE is unavailable.

Both transports have identical semantics: a hint only means "pull needed." It never carries message content, never advances client state, never decides invite outcomes, and never replaces the ordered pull/sync/admission loop. Room hints carry coalescable high-watermark metadata, such as "room advanced to sequence N"; multiple publishes may collapse into one hint, and clients ignore hints at or below their local cursor. This keeps the native app and core actor from baking in polling while preserving the existing tested fallback path.

`FiniteChatRuntime::wait_for_update` is the app-facing boundary. It builds the
watch request from Rust-owned room cursors and invite state, waits on
`/sync/stream`, then performs the normal pull/admission/finalize tick before
returning a new `AppState`. Native apps may run that method on a background
task, but they must not interpret stream events or expose manual sync controls.
