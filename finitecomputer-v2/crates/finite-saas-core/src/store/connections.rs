//! Core-owned inference intent. No agent enrollment, HTTP activation or local
//! configuration adoption is implied by storing a revision here.
use super::{CoreStore, store_error};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum ConnectionsError {
    #[error("connection request is invalid")]
    Invalid,
    #[error("connection is unavailable for this account")]
    Unauthorized,
    #[error("connection revision changed; refresh before saving")]
    RevisionConflict,
    #[error("change id was already used for a different request")]
    ChangeConflict,
    #[error("connection credential key is unavailable")]
    KeyUnavailable,
    #[error("connection credential could not be authenticated")]
    CredentialInvalid,
    #[error(transparent)]
    Store(#[from] crate::CoreError),
}
type Result<T> = std::result::Result<T, ConnectionsError>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceProfile {
    FinitePrivate,
    Openrouter,
}
impl InferenceProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::FinitePrivate => "finite_private",
            Self::Openrouter => "openrouter",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "finite_private" => Ok(Self::FinitePrivate),
            "openrouter" => Ok(Self::Openrouter),
            _ => Err(ConnectionsError::Invalid),
        }
    }
}

// Secret input deliberately has neither Debug nor Serialize.
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum InferenceCredentialChange {
    Keep,
    Replace {
        #[serde(deserialize_with = "deserialize_secret")]
        api_key: Zeroizing<String>,
    },
}
fn deserialize_secret<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(d).map(Zeroizing::new)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetInference {
    pub change_id: String,
    pub expected_revision: i64,
    pub profile: InferenceProfile,
    pub model: Option<String>,
    pub credential: InferenceCredentialChange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InferenceSettings {
    pub revision: i64,
    pub profile: InferenceProfile,
    pub model: Option<String>,
    pub credential_version: i64,
}

/// Callers load key material from deployment secrets, never from database rows.
/// Keep old keys available while ciphertext OR retry receipts reference them.
/// Neither the key ring nor secret-bearing requests are printable/serializable.
pub struct ConnectionsKeys {
    active: String,
    keys: BTreeMap<String, Zeroizing<[u8; 32]>>,
}
impl ConnectionsKeys {
    pub fn new(active: String, keys: BTreeMap<String, Zeroizing<[u8; 32]>>) -> Result<Self> {
        if !valid_id(&active) || !keys.contains_key(&active) || keys.keys().any(|k| !valid_id(k)) {
            return Err(ConnectionsError::Invalid);
        }
        Ok(Self { active, keys })
    }
    fn key(&self, id: &str) -> Result<&[u8; 32]> {
        self.keys
            .get(id)
            .map(|k| &**k)
            .ok_or(ConnectionsError::KeyUnavailable)
    }
    fn fingerprint(
        &self,
        key_id: &str,
        project: &str,
        actor: &str,
        input: &SetInference,
    ) -> Result<Vec<u8>> {
        let mut derive = <Hmac<Sha256> as Mac>::new_from_slice(self.key(key_id)?)
            .map_err(|_| ConnectionsError::Invalid)?;
        derive.update(b"finite.connections.idempotency-key.v1");
        let fingerprint_key = Zeroizing::new(<[u8; 32]>::from(derive.finalize().into_bytes()));
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&*fingerprint_key)
            .map_err(|_| ConnectionsError::Invalid)?;
        // Length-prefix every field, including optionality, to avoid ambiguous encodings.
        for field in [
            b"finite.connections.inference.change.v1".as_slice(),
            project.as_bytes(),
            actor.as_bytes(),
            input.change_id.as_bytes(),
            &input.expected_revision.to_be_bytes(),
            input.profile.as_str().as_bytes(),
        ] {
            mac.update(&(field.len() as u64).to_be_bytes());
            mac.update(field);
        }
        match &input.model {
            None => mac.update(&[0]),
            Some(model) => {
                mac.update(&[1]);
                mac.update(&(model.len() as u64).to_be_bytes());
                mac.update(model.as_bytes());
            }
        }
        match &input.credential {
            InferenceCredentialChange::Keep => mac.update(&[0]),
            InferenceCredentialChange::Replace { api_key } => {
                mac.update(&[1]);
                mac.update(api_key.as_bytes());
            }
        }
        Ok(mac.finalize().into_bytes().to_vec())
    }
    fn seal(&self, project: &str, version: i64, secret: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut nonce = [0; 24];
        getrandom::getrandom(&mut nonce).map_err(|_| ConnectionsError::CredentialInvalid)?;
        let cipher = XChaCha20Poly1305::new(self.key(&self.active)?.into());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: secret.as_bytes(),
                    aad: &credential_context(project, version),
                },
            )
            .map_err(|_| ConnectionsError::CredentialInvalid)?;
        Ok((nonce.to_vec(), ciphertext))
    }
}

fn credential_context(project: &str, version: i64) -> Vec<u8> {
    // JSON tuple provides unambiguous versioned domain/project/kind/version binding.
    serde_json::to_vec(&(
        "finite.connections.credential.v1",
        project,
        "inference.openrouter",
        version,
    ))
    .expect("string tuple serializes")
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn validate(input: &SetInference) -> Result<()> {
    if !valid_id(&input.change_id)
        || input.expected_revision < 0
        || input.expected_revision == i64::MAX
        || (input.profile == InferenceProfile::FinitePrivate && input.model.is_some())
        || input.model.as_ref().is_some_and(|m| {
            m.is_empty() || m.len() > 256 || m.trim() != m || m.chars().any(char::is_control)
        })
    {
        return Err(ConnectionsError::Invalid);
    }
    if let InferenceCredentialChange::Replace { api_key } = &input.credential
        && (input.profile != InferenceProfile::Openrouter
            || api_key.is_empty()
            || api_key.len() > 4096
            || api_key.trim() != api_key.as_str()
            || api_key.chars().any(char::is_control))
    {
        return Err(ConnectionsError::Invalid);
    }
    Ok(())
}
fn settings(row: &tokio_postgres::Row) -> Result<InferenceSettings> {
    Ok(InferenceSettings {
        revision: row.get("revision"),
        profile: InferenceProfile::parse(row.get("profile"))?,
        model: row.get("model"),
        credential_version: row.get("credential_version"),
    })
}

impl CoreStore {
    /// Caller supplies a VERIFIED WorkOS subject. Authorization is resolved from
    /// Core on every operation; a formerly authorized retry does not bypass it.
    pub async fn set_connection_inference(
        &self,
        workos_subject: &str,
        project: &str,
        input: SetInference,
        keys: &ConnectionsKeys,
    ) -> Result<InferenceSettings> {
        validate(&input)?;
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        // Serialize the initial insert as well as updates, and fence ownership
        // changes on the Project row in the same transaction as the write.
        let actor = tx
            .query_opt(
                "SELECT p.owner_user_id FROM projects p
             JOIN users u ON u.id = p.owner_user_id
             WHERE p.id = $1 AND u.workos_user_id = $2 FOR UPDATE OF p",
                &[&project, &workos_subject],
            )
            .await
            .map_err(store_error)?
            .ok_or(ConnectionsError::Unauthorized)?
            .get::<_, String>(0);
        if let Some(receipt) = tx
            .query_opt(
                "SELECT * FROM connection_inference_changes
             WHERE project_id = $1 AND change_id = $2",
                &[&project, &input.change_id],
            )
            .await
            .map_err(store_error)?
        {
            let fingerprint =
                keys.fingerprint(receipt.get("fingerprint_key_id"), project, &actor, &input)?;
            let recorded: Vec<u8> = receipt.get("fingerprint");
            if !bool::from(recorded.ct_eq(&fingerprint)) {
                return Err(ConnectionsError::ChangeConflict);
            }
            let result = settings(&receipt)?;
            self.finish(tx).await?;
            return Ok(result);
        }
        let previous = tx
            .query_opt(
                "SELECT * FROM connection_inference_settings WHERE project_id = $1",
                &[&project],
            )
            .await
            .map_err(store_error)?;
        let revision = previous.as_ref().map_or(0, |r| r.get::<_, i64>("revision"));
        if input.expected_revision != revision {
            return Err(ConnectionsError::RevisionConflict);
        }
        let mut version = previous
            .as_ref()
            .map_or(0, |r| r.get::<_, i64>("credential_version"));
        let mut key_id: Option<String> = previous.as_ref().and_then(|r| r.get("credential_key_id"));
        let mut nonce: Option<Vec<u8>> = previous.as_ref().and_then(|r| r.get("credential_nonce"));
        let mut ciphertext: Option<Vec<u8>> = previous
            .as_ref()
            .and_then(|r| r.get("credential_ciphertext"));
        if let InferenceCredentialChange::Replace { api_key } = &input.credential {
            version = version.checked_add(1).ok_or(ConnectionsError::Invalid)?;
            let sealed = keys.seal(project, version, api_key)?;
            key_id = Some(keys.active.clone());
            nonce = Some(sealed.0);
            ciphertext = Some(sealed.1);
        }
        // Keep may refer to an existing local credential on first adoption.
        // The agent must validate that precondition; Core cannot claim applied.
        let result = InferenceSettings {
            revision: revision + 1,
            profile: input.profile,
            model: input.model.clone(),
            credential_version: version,
        };
        let fingerprint = keys.fingerprint(&keys.active, project, &actor, &input)?;
        tx.execute(
            "INSERT INTO connection_inference_settings
               (project_id, revision, profile, model, credential_version,
                credential_key_id, credential_nonce, credential_ciphertext)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (project_id) DO UPDATE SET
               revision=EXCLUDED.revision, profile=EXCLUDED.profile,
               model=EXCLUDED.model, credential_version=EXCLUDED.credential_version,
               credential_key_id=EXCLUDED.credential_key_id,
               credential_nonce=EXCLUDED.credential_nonce,
               credential_ciphertext=EXCLUDED.credential_ciphertext, updated_at=now()",
            &[
                &project,
                &result.revision,
                &input.profile.as_str(),
                &input.model,
                &version,
                &key_id,
                &nonce,
                &ciphertext,
            ],
        )
        .await
        .map_err(store_error)?;
        tx.execute(
            "INSERT INTO connection_inference_changes
               (project_id, change_id, actor_user_id, fingerprint_key_id,
                fingerprint, revision, profile, model, credential_version)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[
                &project,
                &input.change_id,
                &actor,
                &keys.active,
                &fingerprint,
                &result.revision,
                &input.profile.as_str(),
                &input.model,
                &version,
            ],
        )
        .await
        .map_err(store_error)?;
        self.finish(tx).await?;
        Ok(result)
    }

    /// Redacted desired state only. Absence means unmanaged, never disconnect.
    pub async fn connection_inference(
        &self,
        workos_subject: &str,
        project: &str,
    ) -> Result<Option<InferenceSettings>> {
        let client = self.connection().await?;
        let row = client
            .query_opt(
                "SELECT s.revision, s.profile, s.model, s.credential_version
             FROM projects p JOIN users u ON u.id=p.owner_user_id
             LEFT JOIN connection_inference_settings s ON s.project_id=p.id
             WHERE p.id=$1 AND u.workos_user_id=$2",
                &[&project, &workos_subject],
            )
            .await
            .map_err(store_error)?
            .ok_or(ConnectionsError::Unauthorized)?;
        if row.get::<_, Option<i64>>("revision").is_none() {
            return Ok(None);
        }
        settings(&row).map(Some)
    }
}

#[cfg(test)]
mod tests;
