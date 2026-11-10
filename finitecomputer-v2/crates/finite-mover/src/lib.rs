//! `finite-mover`: moves finite.computer agents to finite.chat (FIN-463).
//! It runs on lat-2, only calls out, and acts only on finite.chat's
//! commands: it pushes the roster, reads its mailbox, and does each
//! command with Core's own CLI, the runner hosts' reader and its stage
//! uploader, reporting each step (`mover`).
//!
//! Delete it with the rest of the move (FIN-463's removal plan).

pub mod child;
pub mod config;
pub mod contract;
pub mod core_cli;
pub mod fleet;
pub mod hosts;
pub mod migrations;
pub mod mover;
pub mod sink;
pub mod stage;
pub mod zip_scan;
pub mod zip_tail;
