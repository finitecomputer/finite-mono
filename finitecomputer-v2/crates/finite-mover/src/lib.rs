//! `finite-mover`: moves finite.computer agents to finite.chat (FIN-463).
//! It runs on lat-2, only calls out, and acts only on finite.chat's
//! commands. This crate so far holds its stage uploader; the roster and
//! the client of finite.chat's Migrations follow.
//!
//! Delete it with the rest of the move (FIN-463's removal plan).

pub mod bucket;
pub mod s3;
pub mod stage;
pub mod zip_tail;
