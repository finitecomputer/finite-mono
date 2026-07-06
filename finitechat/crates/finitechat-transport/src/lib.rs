//! Shared Finite Chat transport value types.
//!
//! These are deliberately small, serializable boundary types used by the HTTP
//! delivery service, CLI, client runtime adapter, and server persistence layer.

pub mod engine;
pub mod transport;
pub mod types;

pub use engine::{KeyPackage, KeyPackageSource};
pub use transport::{Timestamp, TransportEnvelope, TransportMessage, TransportSource};
pub use types::{EpochId, GroupId, MemberId, MessageId};
