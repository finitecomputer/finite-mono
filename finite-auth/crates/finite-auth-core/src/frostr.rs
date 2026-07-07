use std::collections::BTreeSet;
use std::fmt;

use finite_nostr::NostrPublicKey;

use crate::AuthError;

/// First Finite Frostr policy: two shares can sign.
pub const FINITE_FROSTR_THRESHOLD: u8 = 2;
/// First Finite Frostr policy: three total shares.
pub const FINITE_FROSTR_MEMBER_COUNT: u8 = 3;

const MIN_SHARE_PACKAGE_REF_LEN: usize = 8;
const MAX_SHARE_PACKAGE_REF_LEN: usize = 256;

/// Fixed share placement role for the first Finite Frostr setup.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum FrostrShareRole {
    /// Share held by the Finite auth server.
    Server,
    /// Share held by the user's normal client.
    UserClient,
    /// Share held in native keychain or secure storage.
    NativeSecureStorage,
}

impl FrostrShareRole {
    /// Stable storage representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::UserClient => "user_client",
            Self::NativeSecureStorage => "native_secure_storage",
        }
    }

    /// Parse a stable storage representation.
    pub fn parse(value: &str) -> Result<Self, AuthError> {
        match value {
            "server" => Ok(Self::Server),
            "user_client" => Ok(Self::UserClient),
            "native_secure_storage" => Ok(Self::NativeSecureStorage),
            _ => Err(AuthError::InvalidInput {
                field: "frostr_share_role",
                reason: "expected server, user_client, or native_secure_storage".to_string(),
            }),
        }
    }
}

impl fmt::Display for FrostrShareRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reference to a share package outside finite-auth core.
///
/// This is intentionally a bounded handle, not decrypted share material. The
/// concrete package format belongs to bifrost-rs or a platform storage adapter.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FrostrSharePackageRef(String);

impl FrostrSharePackageRef {
    /// Validate and create a share package reference.
    pub fn new(value: impl Into<String>) -> Result<Self, AuthError> {
        let value = value.into();
        let len = value.len();
        if len < MIN_SHARE_PACKAGE_REF_LEN {
            return Err(AuthError::InvalidInput {
                field: "frostr_share_package_ref",
                reason: format!("expected at least {MIN_SHARE_PACKAGE_REF_LEN} characters"),
            });
        }
        if len > MAX_SHARE_PACKAGE_REF_LEN {
            return Err(AuthError::LimitExceeded {
                field: "frostr_share_package_ref",
                limit: MAX_SHARE_PACKAGE_REF_LEN,
                actual: len,
            });
        }

        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        {
            return Err(AuthError::InvalidInput {
                field: "frostr_share_package_ref",
                reason: "expected ASCII letters, digits, dash, underscore, dot, or colon"
                    .to_string(),
            });
        }

        Ok(Self(value))
    }

    /// Borrow the package reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FrostrSharePackageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One share placement in the fixed Finite Frostr setup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FrostrSharePlacement {
    role: FrostrShareRole,
    member_index: u8,
    package_ref: FrostrSharePackageRef,
}

impl FrostrSharePlacement {
    /// Create a placement for one Frostr member share.
    pub fn new(
        role: FrostrShareRole,
        member_index: u8,
        package_ref: FrostrSharePackageRef,
    ) -> Result<Self, AuthError> {
        if !(1..=FINITE_FROSTR_MEMBER_COUNT).contains(&member_index) {
            return Err(AuthError::InvalidInput {
                field: "frostr_member_index",
                reason: "expected member index in 1..=3".to_string(),
            });
        }

        Ok(Self {
            role,
            member_index,
            package_ref,
        })
    }

    /// Placement role.
    pub fn role(&self) -> FrostrShareRole {
        self.role
    }

    /// FROSTR member index.
    pub fn member_index(&self) -> u8 {
        self.member_index
    }

    /// Package reference for the share at this placement.
    pub fn package_ref(&self) -> &FrostrSharePackageRef {
        &self.package_ref
    }
}

/// Fixed 2-of-3 Frostr keyset placement plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FrostrKeysetPlan {
    group_public_key: NostrPublicKey,
    shares: Vec<FrostrSharePlacement>,
}

impl FrostrKeysetPlan {
    /// Create and validate a keyset plan for the first Finite Frostr policy.
    pub fn new(
        group_public_key: NostrPublicKey,
        shares: Vec<FrostrSharePlacement>,
    ) -> Result<Self, AuthError> {
        validate_share_set(&shares)?;
        Ok(Self {
            group_public_key,
            shares,
        })
    }

    /// Group public key. This is the user's primary Finite/Nostr identity.
    pub fn group_public_key(&self) -> NostrPublicKey {
        self.group_public_key
    }

    /// Fixed threshold.
    pub fn threshold(&self) -> u8 {
        FINITE_FROSTR_THRESHOLD
    }

    /// Fixed member count.
    pub fn member_count(&self) -> u8 {
        FINITE_FROSTR_MEMBER_COUNT
    }

    /// Share placements.
    pub fn shares(&self) -> &[FrostrSharePlacement] {
        &self.shares
    }

    /// Find the share placement for a role.
    pub fn share_for_role(&self, role: FrostrShareRole) -> Option<&FrostrSharePlacement> {
        self.shares.iter().find(|share| share.role == role)
    }
}

/// Durable keyset lifecycle state.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum FrostrKeysetStatus {
    /// Ceremony has started or metadata has been recorded but signing is not ready.
    Pending,
    /// Keyset can be used by the runtime.
    Active,
    /// A replacement share set is being prepared.
    Rotating,
    /// Keyset should no longer participate in signing.
    Disabled,
}

impl FrostrKeysetStatus {
    /// Stable storage representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Rotating => "rotating",
            Self::Disabled => "disabled",
        }
    }

    /// Parse a stable storage representation.
    pub fn parse(value: &str) -> Result<Self, AuthError> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "rotating" => Ok(Self::Rotating),
            "disabled" => Ok(Self::Disabled),
            _ => Err(AuthError::InvalidInput {
                field: "frostr_keyset_status",
                reason: "expected pending, active, rotating, or disabled".to_string(),
            }),
        }
    }
}

impl fmt::Display for FrostrKeysetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stored Frostr keyset metadata without decrypted share material.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FrostrKeysetRecord {
    plan: FrostrKeysetPlan,
    status: FrostrKeysetStatus,
    created_at_unix_seconds: u64,
    activated_at_unix_seconds: Option<u64>,
}

impl FrostrKeysetRecord {
    /// Create a keyset metadata record.
    pub fn new(
        plan: FrostrKeysetPlan,
        status: FrostrKeysetStatus,
        created_at_unix_seconds: u64,
    ) -> Result<Self, AuthError> {
        if status != FrostrKeysetStatus::Pending {
            return Err(AuthError::InvalidInput {
                field: "frostr_keyset_status",
                reason: "new keyset records must start pending".to_string(),
            });
        }

        Ok(Self {
            plan,
            status,
            created_at_unix_seconds,
            activated_at_unix_seconds: None,
        })
    }

    /// Mark the keyset active at a specific time.
    pub fn activated_at(mut self, activated_at_unix_seconds: u64) -> Result<Self, AuthError> {
        if activated_at_unix_seconds < self.created_at_unix_seconds {
            return Err(AuthError::InvalidInput {
                field: "frostr_keyset_activated_at",
                reason: "activated_at must be after created_at".to_string(),
            });
        }

        self.status = FrostrKeysetStatus::Active;
        self.activated_at_unix_seconds = Some(activated_at_unix_seconds);
        Ok(self)
    }

    /// Change status after activation while preserving the original activation time.
    pub fn with_status(mut self, status: FrostrKeysetStatus) -> Result<Self, AuthError> {
        let activation_present = self.activated_at_unix_seconds.is_some();
        if status == FrostrKeysetStatus::Pending && activation_present {
            return Err(AuthError::InvalidInput {
                field: "frostr_keyset_status",
                reason: "activated keysets cannot return to pending".to_string(),
            });
        }
        if status != FrostrKeysetStatus::Pending && !activation_present {
            return Err(AuthError::InvalidInput {
                field: "frostr_keyset_status",
                reason: "non-pending keysets require activated_at".to_string(),
            });
        }

        self.status = status;
        Ok(self)
    }

    /// Keyset placement plan.
    pub fn plan(&self) -> &FrostrKeysetPlan {
        &self.plan
    }

    /// Keyset status.
    pub fn status(&self) -> FrostrKeysetStatus {
        self.status
    }

    /// Creation time.
    pub fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }

    /// Activation time.
    pub fn activated_at_unix_seconds(&self) -> Option<u64> {
        self.activated_at_unix_seconds
    }
}

/// Agent Nostr key delegated to act for a user primary key.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgentNostrKeyBinding {
    user_public_key: NostrPublicKey,
    agent_public_key: NostrPublicKey,
    created_at_unix_seconds: u64,
    revoked_at_unix_seconds: Option<u64>,
}

impl AgentNostrKeyBinding {
    /// Create a delegated agent key binding.
    pub fn new(
        user_public_key: NostrPublicKey,
        agent_public_key: NostrPublicKey,
        created_at_unix_seconds: u64,
    ) -> Result<Self, AuthError> {
        if user_public_key == agent_public_key {
            return Err(AuthError::InvalidInput {
                field: "agent_public_key",
                reason: "agent key must differ from user primary key".to_string(),
            });
        }

        Ok(Self {
            user_public_key,
            agent_public_key,
            created_at_unix_seconds,
            revoked_at_unix_seconds: None,
        })
    }

    /// Mark this binding revoked.
    pub fn revoked_at(mut self, revoked_at_unix_seconds: u64) -> Result<Self, AuthError> {
        if revoked_at_unix_seconds < self.created_at_unix_seconds {
            return Err(AuthError::InvalidInput {
                field: "agent_key_revoked_at",
                reason: "revoked_at must be after created_at".to_string(),
            });
        }

        self.revoked_at_unix_seconds = Some(revoked_at_unix_seconds);
        Ok(self)
    }

    /// User primary public key.
    pub fn user_public_key(&self) -> NostrPublicKey {
        self.user_public_key
    }

    /// Delegated agent public key.
    pub fn agent_public_key(&self) -> NostrPublicKey {
        self.agent_public_key
    }

    /// Creation time.
    pub fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }

    /// Revocation time.
    pub fn revoked_at_unix_seconds(&self) -> Option<u64> {
        self.revoked_at_unix_seconds
    }

    /// Whether the binding is revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked_at_unix_seconds.is_some()
    }
}

fn validate_share_set(shares: &[FrostrSharePlacement]) -> Result<(), AuthError> {
    if shares.len() != FINITE_FROSTR_MEMBER_COUNT as usize {
        return Err(AuthError::InvalidInput {
            field: "frostr_share_roles",
            reason: "expected server, user_client, and native_secure_storage shares".to_string(),
        });
    }

    let roles = shares
        .iter()
        .map(FrostrSharePlacement::role)
        .collect::<BTreeSet<_>>();
    let expected_roles = BTreeSet::from([
        FrostrShareRole::Server,
        FrostrShareRole::UserClient,
        FrostrShareRole::NativeSecureStorage,
    ]);
    if roles != expected_roles {
        return Err(AuthError::InvalidInput {
            field: "frostr_share_roles",
            reason: "expected server, user_client, and native_secure_storage shares".to_string(),
        });
    }

    let indexes = shares
        .iter()
        .map(FrostrSharePlacement::member_index)
        .collect::<BTreeSet<_>>();
    if indexes.len() != FINITE_FROSTR_MEMBER_COUNT as usize {
        return Err(AuthError::InvalidInput {
            field: "frostr_member_indexes",
            reason: "expected unique member indexes".to_string(),
        });
    }

    Ok(())
}
