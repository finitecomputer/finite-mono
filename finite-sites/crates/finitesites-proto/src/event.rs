//! NIP-01 event encoding, signing, and verification lives in the shared
//! `finite-authn` crate. This module is the compatibility surface for
//! existing `finitesites_proto::event` consumers.

pub use finite_authn::event::{NostrEvent, pubkey_for_secret};
