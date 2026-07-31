//! Identity Authority client for product-owned authorization checks.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct IdentityAuthority {
    base_url: String,
}

impl IdentityAuthority {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub fn satisfies_grant(&self, grant: &str, actor_pubkey: &str) -> Result<bool, String> {
        let url = format!(
            "{}/api/v1/principal-resolution/satisfies-grant",
            self.base_url
        );
        let request = PrincipalResolutionRequest {
            grant,
            actor_pubkey,
        };
        let response = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(request).expect("request serializes"))
            .map_err(|error| format!("identity authority request failed: {error}"))?;
        let response: PrincipalResolutionResponse = response
            .into_json()
            .map_err(|error| format!("identity authority returned invalid json: {error}"))?;
        Ok(response.satisfied)
    }
}

#[derive(Debug, Serialize)]
struct PrincipalResolutionRequest<'a> {
    grant: &'a str,
    actor_pubkey: &'a str,
}

#[derive(Debug, Deserialize)]
struct PrincipalResolutionResponse {
    satisfied: bool,
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn satisfies_grant_posts_to_identity_authority() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            assert_eq!(
                request_line.trim_end(),
                "POST /api/v1/principal-resolution/satisfies-grant HTTP/1.1"
            );

            let mut content_length = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(value) = trimmed.strip_prefix("Content-Length: ") {
                    content_length = Some(value.parse::<usize>().unwrap());
                }
            }
            let mut body = vec![0; content_length.expect("content-length")];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["grant"], "skyler@example.com");
            assert_eq!(body["actor_pubkey"], "11".repeat(32));

            let response = b"{\"satisfied\":true}";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                response.len()
            )
            .unwrap();
            stream.write_all(response).unwrap();
        });

        let authority = IdentityAuthority::new(format!("http://{address}"));
        assert!(
            authority
                .satisfies_grant("skyler@example.com", &"11".repeat(32))
                .unwrap()
        );
        server.join().unwrap();
    }
}
