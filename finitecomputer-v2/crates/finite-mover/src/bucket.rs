//! The few object-store calls the mover makes on its stage bucket. [`crate::s3`]
//! speaks them to R2 (or any S3-compatible store); tests use a fake.

/// One uploaded part of a multipart upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub number: u32,
    /// The part's ETag without quotes: its MD5 in hex on R2 and S3 for
    /// unencrypted objects.
    pub etag: String,
    pub size: u64,
}

/// A multipart upload in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub key: String,
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BucketError {
    /// The store answered with an error. `code` is S3's error code; the
    /// body is never kept, since a store could echo a request.
    #[error("the bucket answered {status} {code} to {operation}")]
    Refused {
        operation: &'static str,
        status: u16,
        code: String,
    },
    #[error("the bucket could not be reached for {operation}: {reason}")]
    Unreachable {
        operation: &'static str,
        reason: String,
    },
    #[error("the bucket's answer to {operation} could not be read")]
    Unreadable { operation: &'static str },
}

pub trait Bucket {
    /// Every key under `prefix`.
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, BucketError>;
    /// A whole small object, or `None` if there is none.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BucketError>;
    fn put(&self, key: &str, body: &[u8], content_type: &str) -> Result<(), BucketError>;
    /// Multipart uploads in progress under `prefix`.
    fn uploads(&self, prefix: &str) -> Result<Vec<Upload>, BucketError>;
    fn create_upload(&self, key: &str) -> Result<String, BucketError>;
    /// The parts uploaded so far, in order.
    fn parts(&self, key: &str, upload_id: &str) -> Result<Vec<Part>, BucketError>;
    /// Upload (or replace) one part; returns its ETag without quotes.
    fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        number: u32,
        body: &[u8],
    ) -> Result<String, BucketError>;
    fn complete(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<(), BucketError>;
    /// Abort an upload and drop its parts.
    fn abort(&self, key: &str, upload_id: &str) -> Result<(), BucketError>;
}
