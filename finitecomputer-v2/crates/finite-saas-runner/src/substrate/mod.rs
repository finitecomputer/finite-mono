//! Substrate owns actor placement, worker assignment and snapshots. Finite
//! retains its existing admission and runtime identity contracts.
mod client;
pub mod ingress;
mod launcher;
pub use launcher::{SubstrateConfig, SubstrateLauncher};

pub use client::{SubstrateClient, SubstrateConnection, SubstrateError};

// Generated from the pinned upstream API; never maintain a second wire model.
#[allow(clippy::all)]
pub mod protocol {
    tonic::include_proto!("ateapi");
}
