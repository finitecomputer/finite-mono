//! The stage bucket over S3's API with AWS Signature Version 4, path-style
//! (`<endpoint>/<bucket>/<key>`), as R2 serves it.
//!
//! Bespoke rather than a crate: the mover needs nine calls, and the small
//! S3 crates bring new major versions of `hmac`, `sha2` and `base64` plus an
//! XML stack into the workspace. SigV4 is checked against AWS's published
//! example, and the whole client against a real S3 implementation
//! (`tests/s3.rs`).
//!
//! The secret key is held in memory only and never printed; errors keep the
//! store's error code, never its body.

use std::time::Duration;

use hmac::{Hmac, Mac};
use reqwest::Url;
use reqwest::blocking::{Client, Response};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::bucket::{Bucket, BucketError, Part, Upload};

/// Where the stage bucket is and how to sign for it.
pub struct S3Config {
    /// `https://<account>.r2.cloudflarestorage.com`, or a local S3.
    pub endpoint: Url,
    pub bucket: String,
    /// `auto` for R2.
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: Zeroizing<String>,
}

pub struct S3Bucket {
    config: S3Config,
    client: Client,
}

impl S3Bucket {
    pub fn new(config: S3Config) -> Result<Self, BucketError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|error| unreachable_error("setup", &error))?;
        Ok(Self { config, client })
    }

    /// Create the bucket: for a local S3 in tests. Deployments make theirs
    /// with their lifecycle rules.
    pub fn create_bucket(&self) -> Result<(), BucketError> {
        let response = self.send(
            "create bucket",
            reqwest::Method::PUT,
            "",
            &[],
            Vec::new(),
            None,
        )?;
        self.expect_ok("create bucket", response).map(drop)
    }

    fn url(&self, key: &str, query: &[(&str, String)]) -> (Url, String) {
        let mut path = format!("/{}", encode(&self.config.bucket, false));
        if !key.is_empty() {
            path.push('/');
            path.push_str(&encode(key, true));
        }
        let mut url = self.config.endpoint.clone();
        let base = url.path().trim_end_matches('/').to_string();
        let full_path = format!("{base}{path}");
        url.set_path(&full_path);
        let query = canonical_query(query);
        url.set_query((!query.is_empty()).then_some(query.as_str()));
        (url, full_path)
    }

    fn send(
        &self,
        operation: &'static str,
        method: reqwest::Method,
        key: &str,
        query: &[(&str, String)],
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<Response, BucketError> {
        let (url, path) = self.url(key, query);
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_string(),
        };
        let payload = hex::encode(Sha256::digest(&body));
        let now = time::OffsetDateTime::now_utc();
        let amz_date = amz_date(now);
        let mut headers = vec![
            ("host".to_string(), host),
            ("x-amz-content-sha256".to_string(), payload.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(content_type) = content_type {
            headers.push(("content-type".to_string(), content_type.to_string()));
        }
        let authorization = authorization(
            &Credentials {
                access_key_id: &self.config.access_key_id,
                secret_access_key: &self.config.secret_access_key,
                region: &self.config.region,
            },
            &amz_date,
            &CanonicalRequest {
                method: method.as_str(),
                path: &path,
                query: url.query().unwrap_or_default(),
                headers: &headers,
                payload_sha256: &payload,
            },
        );
        let mut request = self.client.request(method, url).body(body);
        for (name, value) in headers.iter().filter(|(name, _)| name != "host") {
            request = request.header(name, value);
        }
        request
            .header("authorization", authorization)
            .send()
            .map_err(|error| unreachable_error(operation, &error))
    }

    fn expect_ok(
        &self,
        operation: &'static str,
        response: Response,
    ) -> Result<Vec<u8>, BucketError> {
        let status = response.status();
        let body = response
            .bytes()
            .map_err(|error| unreachable_error(operation, &error))?
            .to_vec();
        if status.is_success() {
            return Ok(body);
        }
        Err(BucketError::Refused {
            operation,
            status: status.as_u16(),
            code: xml_text(&body, "Code").unwrap_or_default(),
        })
    }
}

impl Bucket for S3Bucket {
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, BucketError> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![
                ("list-type", "2".to_string()),
                ("prefix", prefix.to_string()),
            ];
            if let Some(token) = &token {
                query.push(("continuation-token", token.clone()));
            }
            let response = self.send("list", reqwest::Method::GET, "", &query, Vec::new(), None)?;
            let body = self.expect_ok("list", response)?;
            for contents in xml_blocks(&body, "Contents") {
                keys.push(
                    xml_text(contents, "Key")
                        .ok_or(BucketError::Unreadable { operation: "list" })?,
                );
            }
            if xml_text(&body, "IsTruncated").as_deref() != Some("true") {
                return Ok(keys);
            }
            token = Some(
                xml_text(&body, "NextContinuationToken")
                    .ok_or(BucketError::Unreadable { operation: "list" })?,
            );
        }
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BucketError> {
        let response = self.send("get", reqwest::Method::GET, key, &[], Vec::new(), None)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        self.expect_ok("get", response).map(Some)
    }

    fn put(&self, key: &str, body: &[u8], content_type: &str) -> Result<(), BucketError> {
        let response = self.send(
            "put",
            reqwest::Method::PUT,
            key,
            &[],
            body.to_vec(),
            Some(content_type),
        )?;
        self.expect_ok("put", response).map(drop)
    }

    fn uploads(&self, prefix: &str) -> Result<Vec<Upload>, BucketError> {
        let mut uploads = Vec::new();
        let mut markers: Option<(String, String)> = None;
        loop {
            let mut query = vec![("uploads", String::new()), ("prefix", prefix.to_string())];
            if let Some((key, id)) = &markers {
                query.push(("key-marker", key.clone()));
                query.push(("upload-id-marker", id.clone()));
            }
            let response = self.send(
                "uploads",
                reqwest::Method::GET,
                "",
                &query,
                Vec::new(),
                None,
            )?;
            let body = self.expect_ok("uploads", response)?;
            for upload in xml_blocks(&body, "Upload") {
                let read = |tag| {
                    xml_text(upload, tag).ok_or(BucketError::Unreadable {
                        operation: "uploads",
                    })
                };
                uploads.push(Upload {
                    key: read("Key")?,
                    id: read("UploadId")?,
                });
            }
            if xml_text(&body, "IsTruncated").as_deref() != Some("true") {
                return Ok(uploads);
            }
            let next = |tag| {
                xml_text(&body, tag).ok_or(BucketError::Unreadable {
                    operation: "uploads",
                })
            };
            markers = Some((next("NextKeyMarker")?, next("NextUploadIdMarker")?));
        }
    }

    fn create_upload(&self, key: &str) -> Result<String, BucketError> {
        let response = self.send(
            "create upload",
            reqwest::Method::POST,
            key,
            &[("uploads", String::new())],
            Vec::new(),
            Some("application/zip"),
        )?;
        let body = self.expect_ok("create upload", response)?;
        xml_text(&body, "UploadId").ok_or(BucketError::Unreadable {
            operation: "create upload",
        })
    }

    fn parts(&self, key: &str, upload_id: &str) -> Result<Vec<Part>, BucketError> {
        let mut parts = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let mut query = vec![("uploadId", upload_id.to_string())];
            if let Some(marker) = &marker {
                query.push(("part-number-marker", marker.clone()));
            }
            let response =
                self.send("parts", reqwest::Method::GET, key, &query, Vec::new(), None)?;
            let body = self.expect_ok("parts", response)?;
            for part in xml_blocks(&body, "Part") {
                let read =
                    |tag| xml_text(part, tag).ok_or(BucketError::Unreadable { operation: "parts" });
                parts.push(Part {
                    number: read("PartNumber")?
                        .parse()
                        .map_err(|_| BucketError::Unreadable { operation: "parts" })?,
                    etag: read("ETag")?.trim_matches('"').to_string(),
                    size: read("Size")?
                        .parse()
                        .map_err(|_| BucketError::Unreadable { operation: "parts" })?,
                });
            }
            if xml_text(&body, "IsTruncated").as_deref() != Some("true") {
                parts.sort_by_key(|part| part.number);
                return Ok(parts);
            }
            marker = Some(
                xml_text(&body, "NextPartNumberMarker")
                    .ok_or(BucketError::Unreadable { operation: "parts" })?,
            );
        }
    }

    fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        number: u32,
        body: &[u8],
    ) -> Result<String, BucketError> {
        let response = self.send(
            "upload part",
            reqwest::Method::PUT,
            key,
            &[
                ("partNumber", number.to_string()),
                ("uploadId", upload_id.to_string()),
            ],
            body.to_vec(),
            None,
        )?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_matches('"').to_string());
        self.expect_ok("upload part", response)?;
        etag.ok_or(BucketError::Unreadable {
            operation: "upload part",
        })
    }

    fn complete(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<(), BucketError> {
        let mut body = String::from("<CompleteMultipartUpload>");
        for (number, etag) in parts {
            body.push_str(&format!(
                "<Part><PartNumber>{number}</PartNumber><ETag>\"{}\"</ETag></Part>",
                xml_escape(etag)
            ));
        }
        body.push_str("</CompleteMultipartUpload>");
        let response = self.send(
            "complete upload",
            reqwest::Method::POST,
            key,
            &[("uploadId", upload_id.to_string())],
            body.into_bytes(),
            Some("application/xml"),
        )?;
        // S3 may answer 200 with an error document in the body.
        let answer = self.expect_ok("complete upload", response)?;
        match xml_text(&answer, "Code") {
            Some(code) => Err(BucketError::Refused {
                operation: "complete upload",
                status: 200,
                code,
            }),
            None => Ok(()),
        }
    }

    fn abort(&self, key: &str, upload_id: &str) -> Result<(), BucketError> {
        let response = self.send(
            "abort upload",
            reqwest::Method::DELETE,
            key,
            &[("uploadId", upload_id.to_string())],
            Vec::new(),
            None,
        )?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.expect_ok("abort upload", response).map(drop)
    }
}

fn unreachable_error(operation: &'static str, error: &reqwest::Error) -> BucketError {
    // reqwest's message names the URL, never a header or body.
    BucketError::Unreachable {
        operation,
        reason: error.to_string(),
    }
}

pub struct Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub region: &'a str,
}

pub struct CanonicalRequest<'a> {
    pub method: &'a str,
    /// The URI-encoded path.
    pub path: &'a str,
    /// The canonical query string.
    pub query: &'a str,
    /// Lowercase names; every header here is signed.
    pub headers: &'a [(String, String)],
    pub payload_sha256: &'a str,
}

/// The `Authorization` header of a SigV4-signed S3 request.
pub fn authorization(
    credentials: &Credentials<'_>,
    amz_date: &str,
    request: &CanonicalRequest<'_>,
) -> String {
    let mut headers: Vec<(&str, String)> = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str(),
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    headers.sort();
    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    let canonical = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed_headers}\n{}",
        request.method, request.path, request.query, request.payload_sha256
    );
    let date = &amz_date[..8];
    let scope = format!("{date}/{}/s3/aws4_request", credentials.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    );
    let secret = Zeroizing::new(format!("AWS4{}", credentials.secret_access_key));
    let mut key = hmac(secret.as_bytes(), date.as_bytes());
    for part in [credentials.region, "s3", "aws4_request"] {
        key = hmac(&key, part.as_bytes());
    }
    let signature = hex::encode(hmac(&key, string_to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    )
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn amz_date(now: time::OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// SigV4's URI encoding: every byte but `A-Za-z0-9-._~` (and `/` in a
/// path) as `%XX`.
pub fn encode(value: &str, keep_slash: bool) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) || (keep_slash && byte == b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn canonical_query(query: &[(&str, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (encode(name, false), encode(value, false)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Each `<tag>...</tag>` block in `xml`, in order. S3's answers are flat
/// enough that no nesting of the same tag occurs.
fn xml_blocks<'a>(xml: &'a [u8], tag: &str) -> Vec<&'a [u8]> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut blocks = Vec::new();
    let mut rest = xml;
    while let Some(start) = find(rest, open.as_bytes()) {
        let after = &rest[start + open.len()..];
        let Some(end) = find(after, close.as_bytes()) else {
            break;
        };
        blocks.push(&after[..end]);
        rest = &after[end + close.len()..];
    }
    blocks
}

/// The text of the first `<tag>` in `xml`, its entities decoded: the five
/// named ones and numeric references (`&#34;` is how Go-written stores
/// quote an ETag).
fn xml_text(xml: &[u8], tag: &str) -> Option<String> {
    let block = xml_blocks(xml, tag).into_iter().next()?;
    let text = std::str::from_utf8(block).ok()?;
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        let entity = &rest[start + 1..];
        let end = entity.find(';')?;
        let name = &entity[..end];
        let character = match name {
            "quot" => '"',
            "apos" => '\'',
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            _ => {
                let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => name.strip_prefix('#')?.parse().ok()?,
                };
                char::from_u32(code)?
            }
        };
        decoded.push(character);
        rest = &entity[end + 1..];
    }
    decoded.push_str(rest);
    Some(decoded)
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's worked example for S3, "GET Object" (Signature Version 4,
    /// "Examples: Signature Calculations").
    #[test]
    fn signs_aws_published_example() {
        let headers = vec![
            (
                "host".to_string(),
                "examplebucket.s3.amazonaws.com".to_string(),
            ),
            ("range".to_string(), "bytes=0-9".to_string()),
            (
                "x-amz-content-sha256".to_string(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            ),
            ("x-amz-date".to_string(), "20130524T000000Z".to_string()),
        ];
        let authorization = authorization(
            &Credentials {
                access_key_id: "AKIAIOSFODNN7EXAMPLE",
                secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                region: "us-east-1",
            },
            "20130524T000000Z",
            &CanonicalRequest {
                method: "GET",
                path: "/test.txt",
                query: "",
                headers: &headers,
                payload_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            },
        );
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn keys_and_queries_are_encoded_as_sigv4_wants() {
        assert_eq!(
            encode("p1/proj_A b/1/stage.zip", true),
            "p1/proj_A%20b/1/stage.zip"
        );
        assert_eq!(encode("a/b", false), "a%2Fb");
        assert_eq!(
            canonical_query(&[("uploadId", "x/y".into()), ("partNumber", "2".into())]),
            "partNumber=2&uploadId=x%2Fy"
        );
        assert_eq!(canonical_query(&[("uploads", String::new())]), "uploads=");
    }

    #[test]
    fn answers_are_read_by_tag() {
        let body = br#"<ListPartsResult><IsTruncated>false</IsTruncated>
            <Part><PartNumber>1</PartNumber><ETag>&quot;abc&quot;</ETag><Size>5</Size></Part>
            <Part><PartNumber>2</PartNumber><ETag>"def"</ETag><Size>6</Size></Part></ListPartsResult>"#;
        let parts = xml_blocks(body, "Part");
        assert_eq!(parts.len(), 2);
        assert_eq!(xml_text(parts[0], "ETag").as_deref(), Some("\"abc\""));
        assert_eq!(xml_text(body, "IsTruncated").as_deref(), Some("false"));
        assert_eq!(xml_text(body, "Missing"), None);
        // Go's encoder writes quotes as numeric references.
        let go = b"<ETag>&#34;006a&#34;</ETag><Key>a&amp;b&#x2F;c</Key>";
        assert_eq!(xml_text(go, "ETag").as_deref(), Some("\"006a\""));
        assert_eq!(xml_text(go, "Key").as_deref(), Some("a&b/c"));
        assert_eq!(xml_text(b"<K>a &bogus</K>", "K"), None);
    }
}
