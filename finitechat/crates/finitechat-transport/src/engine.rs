use crate::types::MessageId;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Transport provenance for an externally published KeyPackage.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyPackageSource {
    pub event_id: MessageId,
}

/// Opaque MLS KeyPackage bytes plus optional transport provenance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyPackage {
    pub bytes: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<KeyPackageSource>,
}

impl KeyPackage {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            source: None,
        }
    }

    pub fn with_source_event_id(bytes: Vec<u8>, event_id: MessageId) -> Self {
        Self {
            bytes,
            source: Some(KeyPackageSource { event_id }),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl PartialEq for KeyPackage {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Eq for KeyPackage {}

impl Hash for KeyPackage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}
