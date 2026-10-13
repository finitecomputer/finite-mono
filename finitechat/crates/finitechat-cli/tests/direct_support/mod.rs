//! Shared harness for the direct-transport integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use finitechat_mls::NOSTR_SECRET_KEY_BYTES;
use finitechat_server::{HttpServerState, http_router};
use nostr::event::FinalizeEvent;
use nostr::{EventBuilder, Keys, Kind, SecretKey, Tag};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const USER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [41; NOSTR_SECRET_KEY_BYTES];
pub const STRANGER_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [42; NOSTR_SECRET_KEY_BYTES];
pub const AGENT_ID: &str = "agent_project_direct_flow";

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn spawn_chat_server(path: &std::path::Path) -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let app = http_router(HttpServerState::from_sqlite_path(path).unwrap());
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
    });
    let url = format!("http://{addr}");
    for _ in 0..100 {
        if reqwest::blocking::get(format!("{url}/health")).is_ok_and(|r| r.status().is_success()) {
            return url;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("chat server did not start");
}

pub fn cli_json(args: &[&str]) -> Value {
    let mut output = Vec::new();
    finitechat_cli::run(args.iter().map(|arg| arg.to_string()), &mut output)
        .unwrap_or_else(|error| panic!("finitechat {args:?} failed: {error}"));
    serde_json::from_slice(&output).unwrap()
}

/// Run `hermes serve` on an ephemeral port in this process; returns its URL.
pub fn spawn_sidecar(agent_home: &str, ready: PathBuf) -> String {
    let home = agent_home.to_owned();
    let ready_arg = ready.display().to_string();
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = finitechat_cli::run(
            [
                "hermes",
                "--agent-home",
                &home,
                "serve",
                "--addr",
                "127.0.0.1:0",
                "--ready-file",
                &ready_arg,
                "--json",
            ]
            .iter()
            .map(|arg| arg.to_string()),
            &mut sink,
        );
    });
    for _ in 0..600 {
        if let Ok(bytes) = std::fs::read(&ready)
            && let Ok(started) = serde_json::from_slice::<Value>(&bytes)
        {
            return started["url"].as_str().unwrap().to_owned();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("sidecar did not become ready");
}

pub struct Person {
    pub keys: Keys,
}

impl Person {
    pub fn new(secret: [u8; NOSTR_SECRET_KEY_BYTES]) -> Self {
        Self {
            keys: Keys::new(SecretKey::from_slice(&secret).unwrap()),
        }
    }

    pub fn account(&self) -> String {
        self.keys.public_key().to_hex()
    }

    pub fn sign(&self, method: &str, path_and_query: &str, body: Option<&[u8]>) -> String {
        let mut tags = vec![
            Tag::parse([
                "u",
                &format!("finitechat-direct://{AGENT_ID}{path_and_query}"),
            ])
            .unwrap(),
            Tag::parse(["method", method]).unwrap(),
        ];
        if let Some(body) = body {
            tags.push(Tag::parse(["payload", &hex_lower(&Sha256::digest(body))]).unwrap());
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        tags.push(Tag::parse(["nonce", &hex_lower(&nonce)]).unwrap());
        let event = EventBuilder::new(Kind::Custom(27_235), "")
            .tags(tags)
            .finalize(&self.keys)
            .unwrap();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&event).unwrap())
    }
}

pub struct Bridge {
    pub url: String,
    pub http: reqwest::blocking::Client,
}

impl Bridge {
    pub fn get(&self, person: &Person, path: &str) -> reqwest::blocking::Response {
        self.http
            .get(format!("{}{path}", self.url))
            .header(
                "x-finite-chat-authorization",
                person.sign("GET", path, None),
            )
            .send()
            .unwrap()
    }

    pub fn post(&self, person: &Person, path: &str, body: &Value) -> reqwest::blocking::Response {
        let bytes = serde_json::to_vec(body).unwrap();
        self.http
            .post(format!("{}{path}", self.url))
            .header("content-type", "application/json")
            .header(
                "x-finite-chat-authorization",
                person.sign("POST", path, Some(&bytes)),
            )
            .body(bytes)
            .send()
            .unwrap()
    }

    pub fn json(&self, response: reqwest::blocking::Response) -> Value {
        let status = response.status();
        let url = response.url().clone();
        let text = response.text().unwrap();
        let body: Value = serde_json::from_str(&text)
            .unwrap_or_else(|_| panic!("{url} {status}: non-JSON body {text:?}"));
        assert!(status.is_success(), "{url} {status}: {body}");
        body
    }

    pub fn loopback(&self, path: &str, body: &Value) -> Value {
        let response = self
            .http
            .post(format!("{}{path}", self.url))
            .json(body)
            .send()
            .unwrap();
        self.json(response)
    }
}

pub fn chat_messages(state: &Value) -> Vec<String> {
    state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}
