//! `finite-mover-host`: the read-only reader of one Agent Runtime's
//! durable tree, `<work root>/kata/<durable_state_id>`, for moving
//! finite.computer agents to finite.chat (FIN-463).
//!
//! The mover on lat-2 runs it on each runner host over ssh, as the forced
//! command of its key, and nothing else does. It has three operations:
//!
//! - `sizes`: bytes and files in each [`classify::Class`];
//! - `stream`: the stage, a zip64 of the `history` and `work` classes and
//!   the allow-listed `config` copy, to stdout;
//! - `secrets`: the Telegram token, OpenRouter key and Nostr key, to
//!   stdout, for the mover's in-memory hand-off to the vault.
//!
//! It reads only, follows no link, and refuses to stage or hand off from a
//! running agent (the chat store's writer lease).
//!
//! Delete it with the rest of the move (FIN-463's removal plan): once every
//! roster agent is moved, withdrawn or expired.

pub mod classify;
pub mod command;
pub mod config_copy;
pub mod error;
pub mod secrets;
pub mod shapes;
pub mod stage;
pub mod summary;
pub mod tree;
pub mod walk;
pub mod zip64;
