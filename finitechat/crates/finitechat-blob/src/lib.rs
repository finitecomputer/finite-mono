use std::collections::{BTreeMap, BTreeSet};
use std::num::TryFromIntError;

use finitechat_proto::{
    AttachmentBlobEncryptionV1, AttachmentBlobMetadataV1, AttachmentBlobReferenceV1,
    FINITECHAT_ATTACHMENT_BLOB_ENCRYPTION_AES256_GCM_V1, FINITECHAT_ATTACHMENT_BLOB_SCHEME_V1,
    MAX_ATTACHMENT_CIPHERTEXT_BYTES, MAX_ATTACHMENT_MIME_TYPE_BYTES,
    MAX_ATTACHMENT_PLAINTEXT_BYTES, ProtocolLimitError, validate_bytes_len,
    validate_bytes_non_empty, validate_string_bytes,
};
use openmls::prelude::{AeadType, OpenMlsCrypto, OpenMlsProvider, OpenMlsRand};
use openmls_rust_crypto::OpenMlsRustCrypto;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const ATTACHMENT_KEY_BYTES: usize = 32;
pub const ATTACHMENT_NONCE_BYTES: usize = 12;
pub const ATTACHMENT_AES_GCM_TAG_BYTES: usize = 16;
pub const BLOB_CIPHERTEXT_CONTENT_TYPE: &str = "application/octet-stream";
pub const BLOSSOM_UPLOAD_METHOD: &str = "PUT";
pub const BLOSSOM_UPLOAD_PATH: &str = "/upload";
pub const BLOSSOM_DOWNLOAD_METHOD: &str = "GET";
pub const FINITE_BLOB_UPLOAD_METHOD: &str = "PUT";
pub const FINITE_BLOB_DOWNLOAD_METHOD: &str = "GET";
pub const MAX_FINITE_BLOB_PRINCIPAL_BYTES: u32 = 128;
pub const MAX_FINITE_BLOB_NAMESPACE_BYTES: u32 = 128;
pub const MAX_FINITE_BLOB_CONTENT_TYPE_BYTES: u32 = 256;
pub const MAX_FINITE_BLOB_CAPABILITY_PATH_BYTES: u32 = 512;
pub const MAX_FINITE_BLOB_NONCE_BYTES: u32 = 128;

const ATTACHMENT_AAD_DOMAIN: &[u8] = b"finitechat.attachment-blob.aad.v1";

const _: () = {
    assert!(ATTACHMENT_KEY_BYTES == 32);
    assert!(ATTACHMENT_NONCE_BYTES == 12);
    assert!(ATTACHMENT_AES_GCM_TAG_BYTES == 16);
    assert!(MAX_ATTACHMENT_CIPHERTEXT_BYTES == MAX_ATTACHMENT_PLAINTEXT_BYTES + 16);
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AttachmentBlobError {
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
    #[error("failed to generate attachment encryption material")]
    Randomness,
    #[error("failed to encrypt attachment blob")]
    Encrypt,
    #[error("failed to decrypt attachment blob")]
    Decrypt,
    #[error("{field} has {actual_chars} hex chars, expected {expected_chars}")]
    InvalidHexLength {
        field: &'static str,
        expected_chars: usize,
        actual_chars: usize,
    },
    #[error("{field} contains invalid hex byte at index {index}")]
    InvalidHexByte { field: &'static str, index: usize },
    #[error("attachment scheme {0:?} is not supported")]
    UnsupportedScheme(String),
    #[error("attachment encryption algorithm {0:?} is not supported")]
    UnsupportedEncryptionAlgorithm(String),
    #[error("blob descriptor hash mismatch: expected {expected}, got {actual}")]
    BlobDescriptorHashMismatch { expected: String, actual: String },
    #[error("blob descriptor size mismatch: expected {expected}, got {actual}")]
    BlobDescriptorSizeMismatch { expected: u64, actual: u64 },
    #[error("attachment ciphertext hash mismatch: expected {expected}, got {actual}")]
    CiphertextHashMismatch { expected: String, actual: String },
    #[error("attachment ciphertext size mismatch: expected {expected}, got {actual}")]
    CiphertextSizeMismatch { expected: u64, actual: u64 },
    #[error("attachment plaintext hash mismatch: expected {expected}, got {actual}")]
    PlaintextHashMismatch { expected: String, actual: String },
    #[error("attachment plaintext size mismatch: expected {expected}, got {actual}")]
    PlaintextSizeMismatch { expected: u64, actual: u64 },
    #[error("attachment AAD length overflow")]
    AadLengthOverflow,
    #[error("blob HTTP response status {status} is not success")]
    HttpStatus { status: u16 },
    #[error(transparent)]
    Store(#[from] BlobStoreError),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BlobStoreError {
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
    #[error("blob object is missing: {url}")]
    Missing { url: String },
    #[error("blob object hash collision for {sha256}")]
    ObjectHashCollision { sha256: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FiniteBlobCapabilityError {
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
    #[error(transparent)]
    Attachment(#[from] AttachmentBlobError),
    #[error("unknown blob principal: {0}")]
    UnknownPrincipal(String),
    #[error("blob product {product:?} is not enabled for principal {principal}")]
    ProductDisabled {
        principal: String,
        product: FiniteBlobProduct,
    },
    #[error("blob principal {0} is expired")]
    PrincipalExpired(String),
    #[error("blob capability expiry must be greater than now")]
    CapabilityExpired,
    #[error("blob capability expiry exceeds principal allowlist expiry")]
    ExpiryBeyondPrincipal,
    #[error("blob size {size_bytes} exceeds limit {limit_bytes}")]
    ByteLimitExceeded { size_bytes: u64, limit_bytes: u64 },
    #[error("blob capability nonce was already used")]
    NonceReplay,
    #[error("blob ref product or namespace does not match request scope")]
    BlobRefScopeMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiniteBlobProduct {
    Chat,
    Sites,
    Brain,
    Blob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiniteBlobCapabilityKind {
    Upload,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteBlobRef {
    pub product: FiniteBlobProduct,
    pub namespace: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub content_type: String,
}

impl FiniteBlobRef {
    pub fn validate_limits(&self) -> Result<(), FiniteBlobCapabilityError> {
        validate_string_bytes(
            "blob_ref.namespace",
            &self.namespace,
            MAX_FINITE_BLOB_NAMESPACE_BYTES,
        )?;
        validate_string_bytes(
            "blob_ref.url",
            &self.url,
            finitechat_proto::MAX_ATTACHMENT_BLOB_URL_BYTES,
        )?;
        decode_hex_fixed::<32>("blob_ref.sha256", &self.sha256)?;
        validate_bytes_non_empty("blob_ref.bytes", self.size_bytes as usize)?;
        validate_string_bytes(
            "blob_ref.content_type",
            &self.content_type,
            MAX_FINITE_BLOB_CONTENT_TYPE_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteBlobCapability {
    pub capability_id: String,
    pub kind: FiniteBlobCapabilityKind,
    pub principal: String,
    pub product: FiniteBlobProduct,
    pub namespace: String,
    pub method: String,
    pub path: String,
    pub expires_at_ms: u64,
    pub max_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteBlobAllowlistEntry {
    pub principal: String,
    pub products: BTreeSet<FiniteBlobProduct>,
    pub max_upload_bytes: u64,
    pub max_download_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteBlobUploadCapabilityRequest {
    pub principal: String,
    pub product: FiniteBlobProduct,
    pub namespace: String,
    pub content_type: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub now_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiniteBlobDownloadCapabilityRequest {
    pub principal: String,
    pub product: FiniteBlobProduct,
    pub namespace: String,
    pub blob: FiniteBlobRef,
    pub now_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FiniteBlobCapabilityIssuer {
    allowlist: BTreeMap<String, FiniteBlobAllowlistEntry>,
    used_nonces: BTreeSet<String>,
}

impl FiniteBlobCapabilityIssuer {
    pub fn new(entries: Vec<FiniteBlobAllowlistEntry>) -> Result<Self, FiniteBlobCapabilityError> {
        let mut allowlist = BTreeMap::new();
        for entry in entries {
            entry.validate_limits()?;
            allowlist.insert(entry.principal.clone(), entry);
        }
        Ok(Self {
            allowlist,
            used_nonces: BTreeSet::new(),
        })
    }

    pub fn issue_upload_capability(
        &mut self,
        request: FiniteBlobUploadCapabilityRequest,
    ) -> Result<FiniteBlobCapability, FiniteBlobCapabilityError> {
        request.validate_limits()?;
        let entry = self.allowed_entry(
            &request.principal,
            request.product,
            request.now_ms,
            request.expires_at_ms,
        )?;
        if request.size_bytes > entry.max_upload_bytes {
            return Err(FiniteBlobCapabilityError::ByteLimitExceeded {
                size_bytes: request.size_bytes,
                limit_bytes: entry.max_upload_bytes,
            });
        }
        self.consume_nonce(&request.principal, request.product, &request.nonce)?;
        let capability_id = capability_id(
            FiniteBlobCapabilityKind::Upload,
            &request.principal,
            request.product,
            &request.namespace,
            &request.nonce,
            request.sha256.as_deref(),
        );
        Ok(FiniteBlobCapability {
            path: format!("/finite-blob/v1/upload/{capability_id}"),
            capability_id,
            kind: FiniteBlobCapabilityKind::Upload,
            principal: request.principal,
            product: request.product,
            namespace: request.namespace,
            method: FINITE_BLOB_UPLOAD_METHOD.to_owned(),
            expires_at_ms: request.expires_at_ms,
            max_bytes: request.size_bytes,
            sha256: request.sha256,
        })
    }

    pub fn issue_download_capability(
        &mut self,
        request: FiniteBlobDownloadCapabilityRequest,
    ) -> Result<FiniteBlobCapability, FiniteBlobCapabilityError> {
        request.validate_limits()?;
        if request.blob.product != request.product || request.blob.namespace != request.namespace {
            return Err(FiniteBlobCapabilityError::BlobRefScopeMismatch);
        }
        let entry = self.allowed_entry(
            &request.principal,
            request.product,
            request.now_ms,
            request.expires_at_ms,
        )?;
        if request.blob.size_bytes > entry.max_download_bytes {
            return Err(FiniteBlobCapabilityError::ByteLimitExceeded {
                size_bytes: request.blob.size_bytes,
                limit_bytes: entry.max_download_bytes,
            });
        }
        self.consume_nonce(&request.principal, request.product, &request.nonce)?;
        let capability_id = capability_id(
            FiniteBlobCapabilityKind::Download,
            &request.principal,
            request.product,
            &request.namespace,
            &request.nonce,
            Some(&request.blob.sha256),
        );
        Ok(FiniteBlobCapability {
            path: format!(
                "/finite-blob/v1/download/{}?cap={capability_id}",
                request.blob.sha256
            ),
            capability_id,
            kind: FiniteBlobCapabilityKind::Download,
            principal: request.principal,
            product: request.product,
            namespace: request.namespace,
            method: FINITE_BLOB_DOWNLOAD_METHOD.to_owned(),
            expires_at_ms: request.expires_at_ms,
            max_bytes: request.blob.size_bytes,
            sha256: Some(request.blob.sha256),
        })
    }

    fn allowed_entry(
        &self,
        principal: &str,
        product: FiniteBlobProduct,
        now_ms: u64,
        expires_at_ms: u64,
    ) -> Result<FiniteBlobAllowlistEntry, FiniteBlobCapabilityError> {
        let entry = self
            .allowlist
            .get(principal)
            .cloned()
            .ok_or_else(|| FiniteBlobCapabilityError::UnknownPrincipal(principal.to_owned()))?;
        if !entry.products.contains(&product) {
            return Err(FiniteBlobCapabilityError::ProductDisabled {
                principal: principal.to_owned(),
                product,
            });
        }
        if let Some(entry_expiry) = entry.expires_at_ms {
            if now_ms >= entry_expiry {
                return Err(FiniteBlobCapabilityError::PrincipalExpired(
                    principal.to_owned(),
                ));
            }
            if expires_at_ms > entry_expiry {
                return Err(FiniteBlobCapabilityError::ExpiryBeyondPrincipal);
            }
        }
        Ok(entry)
    }

    fn consume_nonce(
        &mut self,
        principal: &str,
        product: FiniteBlobProduct,
        nonce: &str,
    ) -> Result<(), FiniteBlobCapabilityError> {
        let nonce_key = format!("{principal}\0{product:?}\0{nonce}");
        if !self.used_nonces.insert(nonce_key) {
            return Err(FiniteBlobCapabilityError::NonceReplay);
        }
        Ok(())
    }
}

impl FiniteBlobAllowlistEntry {
    pub fn validate_limits(&self) -> Result<(), FiniteBlobCapabilityError> {
        validate_principal(&self.principal)?;
        validate_bytes_non_empty("blob.products", self.products.len())?;
        validate_bytes_non_empty("blob.max_upload_bytes", self.max_upload_bytes as usize)?;
        validate_bytes_non_empty("blob.max_download_bytes", self.max_download_bytes as usize)?;
        Ok(())
    }
}

impl FiniteBlobUploadCapabilityRequest {
    pub fn validate_limits(&self) -> Result<(), FiniteBlobCapabilityError> {
        validate_principal(&self.principal)?;
        validate_namespace(&self.namespace)?;
        validate_string_bytes(
            "blob.content_type",
            &self.content_type,
            MAX_FINITE_BLOB_CONTENT_TYPE_BYTES,
        )?;
        validate_bytes_non_empty("blob.size_bytes", self.size_bytes as usize)?;
        if let Some(sha256) = &self.sha256 {
            decode_hex_fixed::<32>("blob.sha256", sha256)?;
        }
        validate_expiry(self.now_ms, self.expires_at_ms)?;
        validate_nonce(&self.nonce)?;
        Ok(())
    }
}

impl FiniteBlobDownloadCapabilityRequest {
    pub fn validate_limits(&self) -> Result<(), FiniteBlobCapabilityError> {
        validate_principal(&self.principal)?;
        validate_namespace(&self.namespace)?;
        self.blob.validate_limits()?;
        validate_expiry(self.now_ms, self.expires_at_ms)?;
        validate_nonce(&self.nonce)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentEncryptionMaterial {
    pub key: [u8; ATTACHMENT_KEY_BYTES],
    pub nonce: [u8; ATTACHMENT_NONCE_BYTES],
}

impl AttachmentEncryptionMaterial {
    pub fn generate() -> Result<Self, AttachmentBlobError> {
        let provider = OpenMlsRustCrypto::default();
        let key: [u8; ATTACHMENT_KEY_BYTES] = provider
            .rand()
            .random_array()
            .map_err(|_| AttachmentBlobError::Randomness)?;
        let nonce: [u8; ATTACHMENT_NONCE_BYTES] = provider
            .rand()
            .random_array()
            .map_err(|_| AttachmentBlobError::Randomness)?;
        assert_eq!(key.len(), ATTACHMENT_KEY_BYTES);
        assert_eq!(nonce.len(), ATTACHMENT_NONCE_BYTES);
        Ok(Self { key, nonce })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAttachmentUpload {
    pub ciphertext: Vec<u8>,
    pub ciphertext_sha256: String,
    pub plaintext_sha256: String,
    pub plaintext_size: u64,
    pub ciphertext_size: u64,
    pub encryption: AttachmentBlobEncryptionV1,
    pub metadata: AttachmentBlobMetadataV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedAttachment {
    pub reference: AttachmentBlobReferenceV1,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedAttachment {
    pub reference: AttachmentBlobReferenceV1,
    pub plaintext: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobPutRequest<'a> {
    pub ciphertext: &'a [u8],
    pub content_type: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlossomUploadHttpRequest<'a> {
    pub method: &'static str,
    pub path: &'static str,
    pub content_type: &'static str,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlossomUploadHttpResponse {
    pub status: u16,
    pub descriptor: BlobDescriptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlossomDownloadHttpRequest<'a> {
    pub method: &'static str,
    pub url: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlossomDownloadHttpResponse<'a> {
    pub status: u16,
    pub body: &'a [u8],
}

impl<'a> BlobPutRequest<'a> {
    pub fn validate_limits(&self) -> Result<(), ProtocolLimitError> {
        validate_bytes_non_empty("blob.ciphertext", self.ciphertext.len())?;
        validate_bytes_len(
            "blob.ciphertext",
            self.ciphertext.len(),
            MAX_ATTACHMENT_CIPHERTEXT_BYTES,
        )?;
        validate_bytes_non_empty("blob.content_type", self.content_type.len())?;
        validate_string_bytes(
            "blob.content_type",
            self.content_type,
            MAX_ATTACHMENT_MIME_TYPE_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedBlobPut {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub content_type: String,
}

pub trait BlobStore {
    fn put_blob(&mut self, request: BlobPutRequest<'_>) -> Result<BlobDescriptor, BlobStoreError>;
    fn get_blob(&self, url: &str) -> Result<Vec<u8>, BlobStoreError>;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MemoryBlobStore {
    objects_by_url: BTreeMap<String, Vec<u8>>,
    puts: Vec<ObservedBlobPut>,
}

impl MemoryBlobStore {
    pub fn last_put(&self) -> Option<&ObservedBlobPut> {
        self.puts.last()
    }

    pub fn object_bytes(&self, url: &str) -> Option<&[u8]> {
        self.objects_by_url.get(url).map(Vec::as_slice)
    }

    #[cfg(test)]
    fn overwrite_blob(&mut self, url: &str, bytes: Vec<u8>) -> Result<(), BlobStoreError> {
        validate_string_bytes(
            "blob.url",
            url,
            finitechat_proto::MAX_ATTACHMENT_BLOB_URL_BYTES,
        )?;
        let Some(existing) = self.objects_by_url.get_mut(url) else {
            return Err(BlobStoreError::Missing {
                url: url.to_string(),
            });
        };
        *existing = bytes;
        Ok(())
    }
}

impl BlobStore for MemoryBlobStore {
    fn put_blob(&mut self, request: BlobPutRequest<'_>) -> Result<BlobDescriptor, BlobStoreError> {
        request.validate_limits()?;
        let sha256 = sha256_hex(request.ciphertext);
        let url = format!("blossom+memory://sha256/{sha256}");
        let size_bytes = request.ciphertext.len() as u64;

        if let Some(existing) = self.objects_by_url.get(&url) {
            if existing.as_slice() != request.ciphertext {
                return Err(BlobStoreError::ObjectHashCollision { sha256 });
            }
        } else {
            self.objects_by_url
                .insert(url.clone(), request.ciphertext.to_vec());
        }

        let descriptor = BlobDescriptor {
            url,
            sha256,
            size_bytes,
        };
        self.puts.push(ObservedBlobPut {
            url: descriptor.url.clone(),
            sha256: descriptor.sha256.clone(),
            size_bytes: descriptor.size_bytes,
            content_type: request.content_type.to_string(),
        });
        assert_eq!(descriptor.size_bytes, size_bytes);
        Ok(descriptor)
    }

    fn get_blob(&self, url: &str) -> Result<Vec<u8>, BlobStoreError> {
        validate_string_bytes(
            "blob.url",
            url,
            finitechat_proto::MAX_ATTACHMENT_BLOB_URL_BYTES,
        )?;
        self.objects_by_url
            .get(url)
            .cloned()
            .ok_or_else(|| BlobStoreError::Missing {
                url: url.to_string(),
            })
    }
}

pub fn prepare_attachment_upload(
    plaintext: &[u8],
    metadata: AttachmentBlobMetadataV1,
) -> Result<PreparedAttachmentUpload, AttachmentBlobError> {
    let material = AttachmentEncryptionMaterial::generate()?;
    prepare_attachment_upload_with_material(plaintext, metadata, material)
}

pub fn prepare_attachment_upload_with_material(
    plaintext: &[u8],
    metadata: AttachmentBlobMetadataV1,
    material: AttachmentEncryptionMaterial,
) -> Result<PreparedAttachmentUpload, AttachmentBlobError> {
    validate_bytes_non_empty("attachment.plaintext", plaintext.len())?;
    validate_bytes_len(
        "attachment.plaintext",
        plaintext.len(),
        MAX_ATTACHMENT_PLAINTEXT_BYTES,
    )?;
    metadata.validate_limits()?;
    let plaintext_sha256 = sha256_hex(plaintext);
    let aad = attachment_aad(&metadata, plaintext.len() as u64)?;
    let provider = OpenMlsRustCrypto::default();
    let ciphertext = provider
        .crypto()
        .aead_encrypt(
            AeadType::Aes256Gcm,
            &material.key,
            plaintext,
            &material.nonce,
            &aad,
        )
        .map_err(|_| AttachmentBlobError::Encrypt)?;
    validate_bytes_len(
        "attachment.ciphertext",
        ciphertext.len(),
        MAX_ATTACHMENT_CIPHERTEXT_BYTES,
    )?;
    assert_eq!(
        ciphertext.len(),
        plaintext.len() + ATTACHMENT_AES_GCM_TAG_BYTES
    );

    let encryption = AttachmentBlobEncryptionV1 {
        algorithm: FINITECHAT_ATTACHMENT_BLOB_ENCRYPTION_AES256_GCM_V1.to_string(),
        key_hex: hex_lower(&material.key),
        nonce_hex: hex_lower(&material.nonce),
    };
    let prepared = PreparedAttachmentUpload {
        ciphertext_sha256: sha256_hex(&ciphertext),
        plaintext_sha256,
        plaintext_size: plaintext.len() as u64,
        ciphertext_size: ciphertext.len() as u64,
        ciphertext,
        encryption,
        metadata,
    };
    assert_eq!(prepared.ciphertext_size as usize, prepared.ciphertext.len());
    Ok(prepared)
}

pub fn finish_attachment_upload(
    prepared: &PreparedAttachmentUpload,
    descriptor: BlobDescriptor,
) -> Result<AttachmentBlobReferenceV1, AttachmentBlobError> {
    if descriptor.sha256 != prepared.ciphertext_sha256 {
        return Err(AttachmentBlobError::BlobDescriptorHashMismatch {
            expected: prepared.ciphertext_sha256.clone(),
            actual: descriptor.sha256,
        });
    }
    if descriptor.size_bytes != prepared.ciphertext_size {
        return Err(AttachmentBlobError::BlobDescriptorSizeMismatch {
            expected: prepared.ciphertext_size,
            actual: descriptor.size_bytes,
        });
    }
    let reference = AttachmentBlobReferenceV1 {
        scheme: FINITECHAT_ATTACHMENT_BLOB_SCHEME_V1.to_string(),
        url: descriptor.url,
        ciphertext_sha256: prepared.ciphertext_sha256.clone(),
        plaintext_sha256: prepared.plaintext_sha256.clone(),
        plaintext_size: prepared.plaintext_size,
        ciphertext_size: prepared.ciphertext_size,
        encryption: prepared.encryption.clone(),
        metadata: prepared.metadata.clone(),
    };
    validate_reference_exact(&reference)?;
    Ok(reference)
}

pub fn prepare_blossom_upload_http_request(
    prepared: &PreparedAttachmentUpload,
) -> Result<BlossomUploadHttpRequest<'_>, AttachmentBlobError> {
    validate_bytes_non_empty("attachment.ciphertext", prepared.ciphertext.len())?;
    validate_bytes_len(
        "attachment.ciphertext",
        prepared.ciphertext.len(),
        MAX_ATTACHMENT_CIPHERTEXT_BYTES,
    )?;
    let actual_hash = sha256_hex(&prepared.ciphertext);
    if actual_hash != prepared.ciphertext_sha256 {
        return Err(AttachmentBlobError::CiphertextHashMismatch {
            expected: prepared.ciphertext_sha256.clone(),
            actual: actual_hash,
        });
    }
    Ok(BlossomUploadHttpRequest {
        method: BLOSSOM_UPLOAD_METHOD,
        path: BLOSSOM_UPLOAD_PATH,
        content_type: BLOB_CIPHERTEXT_CONTENT_TYPE,
        body: &prepared.ciphertext,
    })
}

pub fn finish_blossom_upload_http_response(
    prepared: &PreparedAttachmentUpload,
    response: BlossomUploadHttpResponse,
) -> Result<AttachmentBlobReferenceV1, AttachmentBlobError> {
    validate_http_success(response.status)?;
    finish_attachment_upload(prepared, response.descriptor)
}

pub fn upload_attachment<S: BlobStore>(
    store: &mut S,
    plaintext: &[u8],
    metadata: AttachmentBlobMetadataV1,
) -> Result<UploadedAttachment, AttachmentBlobError> {
    let prepared = prepare_attachment_upload(plaintext, metadata)?;
    let descriptor = store.put_blob(BlobPutRequest {
        ciphertext: &prepared.ciphertext,
        content_type: BLOB_CIPHERTEXT_CONTENT_TYPE,
    })?;
    let reference = finish_attachment_upload(&prepared, descriptor)?;
    Ok(UploadedAttachment {
        reference,
        ciphertext: prepared.ciphertext,
    })
}

pub fn download_attachment<S: BlobStore>(
    store: &S,
    reference: &AttachmentBlobReferenceV1,
) -> Result<DownloadedAttachment, AttachmentBlobError> {
    let ciphertext = store.get_blob(&reference.url)?;
    decrypt_attachment_ciphertext(reference, &ciphertext)
}

pub fn prepare_blossom_download_http_request(
    reference: &AttachmentBlobReferenceV1,
) -> Result<BlossomDownloadHttpRequest<'_>, AttachmentBlobError> {
    validate_reference_exact(reference)?;
    Ok(BlossomDownloadHttpRequest {
        method: BLOSSOM_DOWNLOAD_METHOD,
        url: &reference.url,
    })
}

pub fn finish_blossom_download_http_response(
    reference: &AttachmentBlobReferenceV1,
    response: BlossomDownloadHttpResponse<'_>,
) -> Result<DownloadedAttachment, AttachmentBlobError> {
    validate_http_success(response.status)?;
    decrypt_attachment_ciphertext(reference, response.body)
}

pub fn decrypt_attachment_ciphertext(
    reference: &AttachmentBlobReferenceV1,
    ciphertext: &[u8],
) -> Result<DownloadedAttachment, AttachmentBlobError> {
    let decoded = validate_reference_exact(reference)?;
    validate_bytes_non_empty("attachment.ciphertext", ciphertext.len())?;
    validate_bytes_len(
        "attachment.ciphertext",
        ciphertext.len(),
        MAX_ATTACHMENT_CIPHERTEXT_BYTES,
    )?;
    let actual_ciphertext_size = ciphertext.len() as u64;
    if actual_ciphertext_size != reference.ciphertext_size {
        return Err(AttachmentBlobError::CiphertextSizeMismatch {
            expected: reference.ciphertext_size,
            actual: actual_ciphertext_size,
        });
    }
    let actual_ciphertext_hash = sha256_hex(ciphertext);
    if actual_ciphertext_hash != reference.ciphertext_sha256 {
        return Err(AttachmentBlobError::CiphertextHashMismatch {
            expected: reference.ciphertext_sha256.clone(),
            actual: actual_ciphertext_hash,
        });
    }

    let aad = attachment_aad(&reference.metadata, reference.plaintext_size)?;
    let provider = OpenMlsRustCrypto::default();
    let plaintext = provider
        .crypto()
        .aead_decrypt(
            AeadType::Aes256Gcm,
            &decoded.key,
            ciphertext,
            &decoded.nonce,
            &aad,
        )
        .map_err(|_| AttachmentBlobError::Decrypt)?;
    let actual_plaintext_size = plaintext.len() as u64;
    if actual_plaintext_size != reference.plaintext_size {
        return Err(AttachmentBlobError::PlaintextSizeMismatch {
            expected: reference.plaintext_size,
            actual: actual_plaintext_size,
        });
    }
    let actual_plaintext_hash = sha256_hex(&plaintext);
    if actual_plaintext_hash != reference.plaintext_sha256 {
        return Err(AttachmentBlobError::PlaintextHashMismatch {
            expected: reference.plaintext_sha256.clone(),
            actual: actual_plaintext_hash,
        });
    }
    Ok(DownloadedAttachment {
        reference: reference.clone(),
        plaintext,
    })
}

fn validate_http_success(status: u16) -> Result<(), AttachmentBlobError> {
    if (200..=299).contains(&status) {
        Ok(())
    } else {
        Err(AttachmentBlobError::HttpStatus { status })
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

fn validate_principal(principal: &str) -> Result<(), FiniteBlobCapabilityError> {
    validate_string_bytes("blob.principal", principal, MAX_FINITE_BLOB_PRINCIPAL_BYTES)?;
    validate_bytes_non_empty("blob.principal", principal.len())?;
    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), FiniteBlobCapabilityError> {
    validate_string_bytes("blob.namespace", namespace, MAX_FINITE_BLOB_NAMESPACE_BYTES)?;
    validate_bytes_non_empty("blob.namespace", namespace.len())?;
    Ok(())
}

fn validate_nonce(nonce: &str) -> Result<(), FiniteBlobCapabilityError> {
    validate_string_bytes("blob.nonce", nonce, MAX_FINITE_BLOB_NONCE_BYTES)?;
    validate_bytes_non_empty("blob.nonce", nonce.len())?;
    Ok(())
}

fn validate_expiry(now_ms: u64, expires_at_ms: u64) -> Result<(), FiniteBlobCapabilityError> {
    if expires_at_ms <= now_ms {
        return Err(FiniteBlobCapabilityError::CapabilityExpired);
    }
    Ok(())
}

fn capability_id(
    kind: FiniteBlobCapabilityKind,
    principal: &str,
    product: FiniteBlobProduct,
    namespace: &str,
    nonce: &str,
    sha256: Option<&str>,
) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(b"finite-blob-capability-v1\0");
    input.extend_from_slice(format!("{kind:?}\0{product:?}\0").as_bytes());
    input.extend_from_slice(principal.as_bytes());
    input.push(0);
    input.extend_from_slice(namespace.as_bytes());
    input.push(0);
    input.extend_from_slice(nonce.as_bytes());
    input.push(0);
    if let Some(sha256) = sha256 {
        input.extend_from_slice(sha256.as_bytes());
    }
    sha256_hex(&input)
}

struct DecodedReference {
    key: [u8; ATTACHMENT_KEY_BYTES],
    nonce: [u8; ATTACHMENT_NONCE_BYTES],
}

fn validate_reference_exact(
    reference: &AttachmentBlobReferenceV1,
) -> Result<DecodedReference, AttachmentBlobError> {
    reference.validate_limits()?;
    if reference.scheme != FINITECHAT_ATTACHMENT_BLOB_SCHEME_V1 {
        return Err(AttachmentBlobError::UnsupportedScheme(
            reference.scheme.clone(),
        ));
    }
    if reference.encryption.algorithm != FINITECHAT_ATTACHMENT_BLOB_ENCRYPTION_AES256_GCM_V1 {
        return Err(AttachmentBlobError::UnsupportedEncryptionAlgorithm(
            reference.encryption.algorithm.clone(),
        ));
    }
    decode_hex_fixed::<32>("attachment.ciphertext_sha256", &reference.ciphertext_sha256)?;
    decode_hex_fixed::<32>("attachment.plaintext_sha256", &reference.plaintext_sha256)?;
    let key = decode_hex_fixed::<ATTACHMENT_KEY_BYTES>(
        "attachment.encryption.key_hex",
        &reference.encryption.key_hex,
    )?;
    let nonce = decode_hex_fixed::<ATTACHMENT_NONCE_BYTES>(
        "attachment.encryption.nonce_hex",
        &reference.encryption.nonce_hex,
    )?;
    assert_eq!(key.len(), ATTACHMENT_KEY_BYTES);
    assert_eq!(nonce.len(), ATTACHMENT_NONCE_BYTES);
    Ok(DecodedReference { key, nonce })
}

fn attachment_aad(
    metadata: &AttachmentBlobMetadataV1,
    plaintext_size: u64,
) -> Result<Vec<u8>, AttachmentBlobError> {
    metadata.validate_limits()?;
    let mut aad = Vec::with_capacity(
        ATTACHMENT_AAD_DOMAIN.len()
            + 4
            + metadata.mime_type.len()
            + 4
            + metadata.filename.len()
            + 8
            + 1
            + 8,
    );
    aad.extend_from_slice(ATTACHMENT_AAD_DOMAIN);
    append_len_prefixed(&mut aad, metadata.mime_type.as_bytes())?;
    append_len_prefixed(&mut aad, metadata.filename.as_bytes())?;
    aad.extend_from_slice(&plaintext_size.to_be_bytes());
    match &metadata.dimensions {
        Some(dimensions) => {
            dimensions.validate_limits()?;
            aad.push(1);
            aad.extend_from_slice(&dimensions.width.to_be_bytes());
            aad.extend_from_slice(&dimensions.height.to_be_bytes());
        }
        None => aad.push(0),
    }
    assert!(aad.len() > ATTACHMENT_AAD_DOMAIN.len());
    Ok(aad)
}

fn append_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), AttachmentBlobError> {
    let len = u32::try_from(bytes.len())
        .map_err(|_: TryFromIntError| AttachmentBlobError::AadLengthOverflow)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn decode_hex_fixed<const N: usize>(
    field: &'static str,
    value: &str,
) -> Result<[u8; N], AttachmentBlobError> {
    let expected_chars = N * 2;
    let bytes = value.as_bytes();
    if bytes.len() != expected_chars {
        return Err(AttachmentBlobError::InvalidHexLength {
            field,
            expected_chars,
            actual_chars: bytes.len(),
        });
    }
    let mut out = [0u8; N];
    for (index, byte_out) in out.iter_mut().enumerate() {
        let hex_index = index * 2;
        let high = hex_nibble(field, hex_index, bytes[hex_index])?;
        let low = hex_nibble(field, hex_index + 1, bytes[hex_index + 1])?;
        *byte_out = (high << 4) | low;
    }
    assert_eq!(out.len(), N);
    Ok(out)
}

fn hex_nibble(field: &'static str, index: usize, byte: u8) -> Result<u8, AttachmentBlobError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(AttachmentBlobError::InvalidHexByte { field, index }),
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    assert_eq!(out.len(), bytes.len() * 2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_proto::{DeviceRef, FiniteEnvelope, LogEntryKind, MlsGroupId, RoomId};

    fn metadata() -> AttachmentBlobMetadataV1 {
        AttachmentBlobMetadataV1 {
            mime_type: "image/png".to_string(),
            filename: "cat.png".to_string(),
            dimensions: Some(finitechat_proto::AttachmentDimensionsV1 {
                width: 640,
                height: 480,
            }),
        }
    }

    fn fixed_material() -> AttachmentEncryptionMaterial {
        AttachmentEncryptionMaterial {
            key: [7u8; ATTACHMENT_KEY_BYTES],
            nonce: [9u8; ATTACHMENT_NONCE_BYTES],
        }
    }

    fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        if needle.is_empty() {
            return true;
        }
        haystack
            .windows(needle.len())
            .any(|candidate| candidate == needle)
    }

    #[test]
    fn attachment_encrypts_before_blob_upload_and_hides_plaintext_metadata_from_store() {
        let plaintext = b"this is the visible image payload before encryption";
        let prepared =
            prepare_attachment_upload_with_material(plaintext, metadata(), fixed_material())
                .expect("prepare");
        assert_ne!(prepared.ciphertext, plaintext);
        assert!(!contains_subsequence(&prepared.ciphertext, b"cat.png"));
        assert!(!contains_subsequence(&prepared.ciphertext, b"image/png"));

        let mut store = MemoryBlobStore::default();
        let descriptor = store
            .put_blob(BlobPutRequest {
                ciphertext: &prepared.ciphertext,
                content_type: BLOB_CIPHERTEXT_CONTENT_TYPE,
            })
            .expect("put");
        let reference = finish_attachment_upload(&prepared, descriptor).expect("finish");
        let put = store.last_put().expect("put observation");

        assert_eq!(put.content_type, BLOB_CIPHERTEXT_CONTENT_TYPE);
        assert_ne!(put.content_type, reference.metadata.mime_type);
        assert!(!put.url.contains(&reference.metadata.filename));
        assert!(!put.url.contains(&reference.metadata.mime_type));
        assert_eq!(
            store.object_bytes(&reference.url).expect("object bytes"),
            prepared.ciphertext
        );
    }

    #[test]
    fn attachment_upload_verifies_ciphertext_hash() {
        let prepared =
            prepare_attachment_upload_with_material(b"hello", metadata(), fixed_material())
                .expect("prepare");
        let descriptor = BlobDescriptor {
            url: "blossom+memory://sha256/bad".to_string(),
            sha256: sha256_hex(b"wrong ciphertext"),
            size_bytes: prepared.ciphertext_size,
        };

        let err =
            finish_attachment_upload(&prepared, descriptor).expect_err("descriptor hash mismatch");
        assert!(matches!(
            err,
            AttachmentBlobError::BlobDescriptorHashMismatch { .. }
        ));
    }

    #[test]
    fn attachment_download_verifies_ciphertext_hash_before_decrypt() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let mut corrupted = store
            .object_bytes(&uploaded.reference.url)
            .expect("ciphertext")
            .to_vec();
        corrupted[0] ^= 0x01;
        store
            .overwrite_blob(&uploaded.reference.url, corrupted)
            .expect("overwrite");

        let err =
            download_attachment(&store, &uploaded.reference).expect_err("hash mismatch first");
        assert!(matches!(
            err,
            AttachmentBlobError::CiphertextHashMismatch { .. }
        ));
    }

    #[test]
    fn attachment_download_verifies_plaintext_hash_after_decrypt() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let mut reference = uploaded.reference.clone();
        reference.plaintext_sha256 = sha256_hex(b"different plaintext");

        let err = download_attachment(&store, &reference).expect_err("plaintext hash mismatch");
        assert!(matches!(
            err,
            AttachmentBlobError::PlaintextHashMismatch { .. }
        ));
    }

    #[test]
    fn attachment_rejects_plaintext_over_v1_size_limit() {
        let too_large = vec![0u8; MAX_ATTACHMENT_PLAINTEXT_BYTES as usize + 1];
        let err = prepare_attachment_upload_with_material(&too_large, metadata(), fixed_material())
            .expect_err("oversized plaintext");
        assert!(matches!(
            err,
            AttachmentBlobError::Protocol(ProtocolLimitError::BytesTooLong { .. })
        ));
    }

    #[test]
    fn attachment_reference_metadata_lives_inside_encrypted_application_payload() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let decrypted_payload = serde_json::to_vec(&uploaded.reference).expect("reference json");
        assert!(contains_subsequence(&decrypted_payload, b"cat.png"));
        assert!(contains_subsequence(&decrypted_payload, b"image/png"));

        let server_envelope = FiniteEnvelope {
            room_id: RoomId::from("room_1"),
            mls_group_id: MlsGroupId::from("group_1"),
            epoch: 7,
            sender: DeviceRef::new("alice", "phone"),
            kind: LogEntryKind::Application,
            payload: b"opaque mls ciphertext".to_vec(),
        };
        let server_view = serde_json::to_vec(&server_envelope).expect("server json");
        assert!(!contains_subsequence(&server_view, b"cat.png"));
        assert!(!contains_subsequence(&server_view, b"image/png"));
    }

    #[test]
    fn attachment_roundtrips_through_memory_blob_store() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let downloaded = download_attachment(&store, &uploaded.reference).expect("download");

        assert_eq!(downloaded.plaintext, b"secret bytes");
        assert_eq!(downloaded.reference, uploaded.reference);
    }

    #[test]
    fn blossom_http_upload_request_uses_ciphertext_only() {
        let plaintext = b"do not leak this plaintext over the descriptor boundary";
        let prepared =
            prepare_attachment_upload_with_material(plaintext, metadata(), fixed_material())
                .expect("prepare");

        let request = prepare_blossom_upload_http_request(&prepared).expect("http request");

        assert_eq!(request.method, BLOSSOM_UPLOAD_METHOD);
        assert_eq!(request.path, BLOSSOM_UPLOAD_PATH);
        assert_eq!(request.content_type, BLOB_CIPHERTEXT_CONTENT_TYPE);
        assert_eq!(request.body, prepared.ciphertext.as_slice());
        assert!(!contains_subsequence(request.body, plaintext));
        assert!(!contains_subsequence(request.body, b"cat.png"));
        assert!(!contains_subsequence(request.body, b"image/png"));
    }

    #[test]
    fn blossom_http_upload_request_rejects_tampered_prepared_ciphertext() {
        let mut prepared =
            prepare_attachment_upload_with_material(b"hello", metadata(), fixed_material())
                .expect("prepare");
        prepared.ciphertext[0] ^= 0x01;

        let err = prepare_blossom_upload_http_request(&prepared)
            .expect_err("prepared ciphertext hash mismatch");

        assert!(matches!(
            err,
            AttachmentBlobError::CiphertextHashMismatch { .. }
        ));
    }

    #[test]
    fn blossom_http_upload_response_verifies_descriptor_before_reference() {
        let prepared =
            prepare_attachment_upload_with_material(b"hello", metadata(), fixed_material())
                .expect("prepare");
        let descriptor = BlobDescriptor {
            url: format!("https://blob.example/{}", prepared.ciphertext_sha256),
            sha256: prepared.ciphertext_sha256.clone(),
            size_bytes: prepared.ciphertext_size,
        };
        let reference = finish_blossom_upload_http_response(
            &prepared,
            BlossomUploadHttpResponse {
                status: 201,
                descriptor,
            },
        )
        .expect("finish");

        assert_eq!(reference.ciphertext_sha256, prepared.ciphertext_sha256);
        assert_eq!(reference.ciphertext_size, prepared.ciphertext_size);

        let err = finish_blossom_upload_http_response(
            &prepared,
            BlossomUploadHttpResponse {
                status: 503,
                descriptor: BlobDescriptor {
                    url: "https://blob.example/down".to_string(),
                    sha256: prepared.ciphertext_sha256.clone(),
                    size_bytes: prepared.ciphertext_size,
                },
            },
        )
        .expect_err("bad status");
        assert_eq!(err, AttachmentBlobError::HttpStatus { status: 503 });
    }

    #[test]
    fn blossom_http_upload_response_rejects_descriptor_size_mismatch() {
        let prepared =
            prepare_attachment_upload_with_material(b"hello", metadata(), fixed_material())
                .expect("prepare");

        let err = finish_blossom_upload_http_response(
            &prepared,
            BlossomUploadHttpResponse {
                status: 201,
                descriptor: BlobDescriptor {
                    url: "https://blob.example/wrong-size".to_string(),
                    sha256: prepared.ciphertext_sha256.clone(),
                    size_bytes: prepared.ciphertext_size + 1,
                },
            },
        )
        .expect_err("descriptor size mismatch");

        assert!(matches!(
            err,
            AttachmentBlobError::BlobDescriptorSizeMismatch { .. }
        ));
    }

    #[test]
    fn blossom_http_upload_retries_next_server_after_failure() {
        let plaintext = b"retry this encrypted payload on another blossom server";
        let prepared =
            prepare_attachment_upload_with_material(plaintext, metadata(), fixed_material())
                .expect("prepare");
        let first_request = prepare_blossom_upload_http_request(&prepared).expect("first request");
        let second_request =
            prepare_blossom_upload_http_request(&prepared).expect("second request");

        let first_err = finish_blossom_upload_http_response(
            &prepared,
            BlossomUploadHttpResponse {
                status: 503,
                descriptor: BlobDescriptor {
                    url: "https://blob-a.example/unavailable".to_string(),
                    sha256: prepared.ciphertext_sha256.clone(),
                    size_bytes: prepared.ciphertext_size,
                },
            },
        )
        .expect_err("first server down");
        let reference = finish_blossom_upload_http_response(
            &prepared,
            BlossomUploadHttpResponse {
                status: 201,
                descriptor: BlobDescriptor {
                    url: format!("https://blob-b.example/{}", prepared.ciphertext_sha256),
                    sha256: prepared.ciphertext_sha256.clone(),
                    size_bytes: prepared.ciphertext_size,
                },
            },
        )
        .expect("second server accepts");

        assert_eq!(first_err, AttachmentBlobError::HttpStatus { status: 503 });
        assert_eq!(first_request.body, second_request.body);
        assert_eq!(first_request.content_type, BLOB_CIPHERTEXT_CONTENT_TYPE);
        assert_eq!(second_request.content_type, BLOB_CIPHERTEXT_CONTENT_TYPE);
        assert!(!contains_subsequence(first_request.body, plaintext));
        assert!(reference.url.starts_with("https://blob-b.example/"));
        assert_eq!(reference.ciphertext_sha256, prepared.ciphertext_sha256);
    }

    #[test]
    fn blossom_http_download_verifies_ciphertext_before_decrypt() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let request =
            prepare_blossom_download_http_request(&uploaded.reference).expect("download request");
        assert_eq!(request.method, BLOSSOM_DOWNLOAD_METHOD);
        assert_eq!(request.url, uploaded.reference.url);

        let downloaded = finish_blossom_download_http_response(
            &uploaded.reference,
            BlossomDownloadHttpResponse {
                status: 200,
                body: &uploaded.ciphertext,
            },
        )
        .expect("download");
        assert_eq!(downloaded.plaintext, b"secret bytes");

        let err = finish_blossom_download_http_response(
            &uploaded.reference,
            BlossomDownloadHttpResponse {
                status: 200,
                body: b"not the ciphertext",
            },
        )
        .expect_err("hash mismatch");
        assert!(matches!(
            err,
            AttachmentBlobError::CiphertextSizeMismatch { .. }
                | AttachmentBlobError::CiphertextHashMismatch { .. }
        ));
    }

    #[test]
    fn blossom_http_download_retries_same_reference_after_failure() {
        let plaintext = b"download retry must not change encrypted reference";
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, plaintext, metadata()).expect("upload");
        let first_request =
            prepare_blossom_download_http_request(&uploaded.reference).expect("first request");
        let second_request =
            prepare_blossom_download_http_request(&uploaded.reference).expect("second request");

        let first_err = finish_blossom_download_http_response(
            &uploaded.reference,
            BlossomDownloadHttpResponse {
                status: 503,
                body: b"temporary failure",
            },
        )
        .expect_err("first download fails");
        let downloaded = finish_blossom_download_http_response(
            &uploaded.reference,
            BlossomDownloadHttpResponse {
                status: 200,
                body: &uploaded.ciphertext,
            },
        )
        .expect("retry succeeds");

        assert_eq!(first_err, AttachmentBlobError::HttpStatus { status: 503 });
        assert_eq!(first_request.method, BLOSSOM_DOWNLOAD_METHOD);
        assert_eq!(second_request.method, BLOSSOM_DOWNLOAD_METHOD);
        assert_eq!(first_request.url, second_request.url);
        assert_eq!(second_request.url, uploaded.reference.url);
        assert_eq!(downloaded.plaintext, plaintext);
    }

    #[test]
    fn blossom_http_download_rejects_http_error_before_body_validation() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");

        let err = finish_blossom_download_http_response(
            &uploaded.reference,
            BlossomDownloadHttpResponse {
                status: 404,
                body: b"not the ciphertext",
            },
        )
        .expect_err("http status wins");

        assert_eq!(err, AttachmentBlobError::HttpStatus { status: 404 });
    }

    #[test]
    fn blossom_http_download_request_rejects_unsupported_reference_scheme() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let mut reference = uploaded.reference;
        reference.scheme = "finitechat.attachment.unknown.v9".to_string();

        let err = prepare_blossom_download_http_request(&reference)
            .expect_err("unsupported scheme fails before network");

        assert_eq!(
            err,
            AttachmentBlobError::UnsupportedScheme("finitechat.attachment.unknown.v9".to_string())
        );
    }

    #[test]
    fn attachment_reference_rejects_uppercase_hex() {
        let mut store = MemoryBlobStore::default();
        let uploaded = upload_attachment(&mut store, b"secret bytes", metadata()).expect("upload");
        let mut reference = uploaded.reference;
        reference.encryption.key_hex.make_ascii_uppercase();

        let err = download_attachment(&store, &reference).expect_err("uppercase hex");
        assert!(matches!(err, AttachmentBlobError::InvalidHexByte { .. }));
    }

    #[test]
    fn finite_blob_upload_capability_binds_principal_product_and_scope() {
        let mut issuer = capability_issuer();

        let capability = issuer
            .issue_upload_capability(FiniteBlobUploadCapabilityRequest {
                principal: "npub-alice".to_owned(),
                product: FiniteBlobProduct::Brain,
                namespace: "brain/artifacts".to_owned(),
                content_type: "application/json".to_owned(),
                size_bytes: 512,
                sha256: Some(sha256_hex(b"artifact")),
                now_ms: 1_000,
                expires_at_ms: 2_000,
                nonce: "nonce-upload-1".to_owned(),
            })
            .expect("upload capability");

        assert_eq!(capability.kind, FiniteBlobCapabilityKind::Upload);
        assert_eq!(capability.principal, "npub-alice");
        assert_eq!(capability.product, FiniteBlobProduct::Brain);
        assert_eq!(capability.namespace, "brain/artifacts");
        assert_eq!(capability.method, FINITE_BLOB_UPLOAD_METHOD);
        assert!(capability.path.starts_with("/finite-blob/v1/upload/"));
        assert_eq!(capability.max_bytes, 512);
        assert_eq!(
            capability.sha256.as_deref(),
            Some(sha256_hex(b"artifact").as_str())
        );
    }

    #[test]
    fn finite_blob_capability_rejects_wrong_principal_product_expiry_quota_and_replay() {
        let mut issuer = capability_issuer();

        let wrong_principal = issuer
            .issue_upload_capability(upload_request(
                "npub-mallory",
                FiniteBlobProduct::Brain,
                512,
                1_000,
                2_000,
                "nonce-missing",
            ))
            .expect_err("unknown principal");
        assert!(matches!(
            wrong_principal,
            FiniteBlobCapabilityError::UnknownPrincipal(_)
        ));

        let wrong_product = issuer
            .issue_upload_capability(upload_request(
                "npub-alice",
                FiniteBlobProduct::Sites,
                512,
                1_000,
                2_000,
                "nonce-product",
            ))
            .expect_err("disabled product");
        assert!(matches!(
            wrong_product,
            FiniteBlobCapabilityError::ProductDisabled { .. }
        ));

        let expired_capability = issuer
            .issue_upload_capability(upload_request(
                "npub-alice",
                FiniteBlobProduct::Brain,
                512,
                2_000,
                2_000,
                "nonce-expired-cap",
            ))
            .expect_err("capability expiry");
        assert_eq!(
            expired_capability,
            FiniteBlobCapabilityError::CapabilityExpired
        );

        let beyond_principal = issuer
            .issue_upload_capability(upload_request(
                "npub-alice",
                FiniteBlobProduct::Brain,
                512,
                1_000,
                20_000,
                "nonce-beyond",
            ))
            .expect_err("principal expiry caps capability");
        assert_eq!(
            beyond_principal,
            FiniteBlobCapabilityError::ExpiryBeyondPrincipal
        );

        let oversized = issuer
            .issue_upload_capability(upload_request(
                "npub-alice",
                FiniteBlobProduct::Brain,
                2_048,
                1_000,
                2_000,
                "nonce-quota",
            ))
            .expect_err("quota rejects");
        assert_eq!(
            oversized,
            FiniteBlobCapabilityError::ByteLimitExceeded {
                size_bytes: 2_048,
                limit_bytes: 1_024
            }
        );

        issuer
            .issue_upload_capability(upload_request(
                "npub-alice",
                FiniteBlobProduct::Brain,
                512,
                1_000,
                2_000,
                "nonce-replay",
            ))
            .expect("first nonce use");
        let replay = issuer
            .issue_upload_capability(upload_request(
                "npub-alice",
                FiniteBlobProduct::Brain,
                512,
                1_000,
                2_000,
                "nonce-replay",
            ))
            .expect_err("nonce replay");
        assert_eq!(replay, FiniteBlobCapabilityError::NonceReplay);
    }

    #[test]
    fn finite_blob_download_capability_rejects_product_scope_mismatch() {
        let mut issuer = capability_issuer();
        let mut blob = finite_blob_ref(FiniteBlobProduct::Brain, "brain/artifacts");
        blob.product = FiniteBlobProduct::Sites;

        let mismatch = issuer
            .issue_download_capability(FiniteBlobDownloadCapabilityRequest {
                principal: "npub-alice".to_owned(),
                product: FiniteBlobProduct::Brain,
                namespace: "brain/artifacts".to_owned(),
                blob,
                now_ms: 1_000,
                expires_at_ms: 2_000,
                nonce: "nonce-download-mismatch".to_owned(),
            })
            .expect_err("product mismatch");

        assert_eq!(mismatch, FiniteBlobCapabilityError::BlobRefScopeMismatch);
    }

    #[test]
    fn finite_blob_download_capability_binds_ref_without_bucket_details() {
        let mut issuer = capability_issuer();
        let blob = finite_blob_ref(FiniteBlobProduct::Brain, "brain/artifacts");

        let capability = issuer
            .issue_download_capability(FiniteBlobDownloadCapabilityRequest {
                principal: "npub-alice".to_owned(),
                product: FiniteBlobProduct::Brain,
                namespace: "brain/artifacts".to_owned(),
                blob: blob.clone(),
                now_ms: 1_000,
                expires_at_ms: 2_000,
                nonce: "nonce-download-1".to_owned(),
            })
            .expect("download capability");

        assert_eq!(capability.kind, FiniteBlobCapabilityKind::Download);
        assert_eq!(capability.method, FINITE_BLOB_DOWNLOAD_METHOD);
        assert!(capability.path.starts_with("/finite-blob/v1/download/"));
        assert!(capability.path.contains("?cap="));
        assert_eq!(capability.sha256.as_deref(), Some(blob.sha256.as_str()));
        assert_eq!(capability.max_bytes, blob.size_bytes);
    }

    fn capability_issuer() -> FiniteBlobCapabilityIssuer {
        FiniteBlobCapabilityIssuer::new(vec![FiniteBlobAllowlistEntry {
            principal: "npub-alice".to_owned(),
            products: BTreeSet::from([FiniteBlobProduct::Chat, FiniteBlobProduct::Brain]),
            max_upload_bytes: 1_024,
            max_download_bytes: 4_096,
            expires_at_ms: Some(10_000),
        }])
        .expect("allowlist")
    }

    fn upload_request(
        principal: &str,
        product: FiniteBlobProduct,
        size_bytes: u64,
        now_ms: u64,
        expires_at_ms: u64,
        nonce: &str,
    ) -> FiniteBlobUploadCapabilityRequest {
        FiniteBlobUploadCapabilityRequest {
            principal: principal.to_owned(),
            product,
            namespace: "brain/artifacts".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            size_bytes,
            sha256: None,
            now_ms,
            expires_at_ms,
            nonce: nonce.to_owned(),
        }
    }

    fn finite_blob_ref(product: FiniteBlobProduct, namespace: &str) -> FiniteBlobRef {
        FiniteBlobRef {
            product,
            namespace: namespace.to_owned(),
            url: "finite-blob://local/sha256/artifact".to_owned(),
            sha256: sha256_hex(b"artifact"),
            size_bytes: 2_048,
            content_type: "application/json".to_owned(),
        }
    }
}
