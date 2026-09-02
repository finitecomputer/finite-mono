//! NIP-98 HTTP authorization lives in the shared `finite-authn` crate so
//! every Finite verifier (Sites today; chat, brain, identity, hermes later)
//! applies the same policy table. This module is the compatibility surface
//! for existing `finitesites_proto::nip98` consumers.

pub use finite_authn::nip98::{
    AUTH_SCHEME, MAX_AUTH_HEADER_BYTES, NIP98_KIND, build_auth_header, verify_auth_header,
};
