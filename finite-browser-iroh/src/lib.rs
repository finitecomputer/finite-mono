//! Browser HTTP over Core-admitted Iroh; no durable browser identity.
#![cfg(target_arch = "wasm32")]
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use std::collections::BTreeMap;
use tokio::io::AsyncReadExt;
use wasm_bindgen::prelude::*;
fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}
type Io = tokio::io::Join<RecvStream, SendStream>;

#[wasm_bindgen]
pub struct BrowserPeer {
    endpoint: Endpoint,
    relay: RelayUrl,
}
#[wasm_bindgen]
impl BrowserPeer {
    pub async fn create(relay: String) -> Result<BrowserPeer, JsError> {
        console_error_panic_hook::set_once();
        let relay: RelayUrl = relay.parse().map_err(err)?;
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Custom(relay.clone().into()))
            .bind()
            .await
            .map_err(err)?;
        Ok(Self { endpoint, relay })
    }
    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }
    pub async fn close(&self) {
        self.endpoint.close().await;
    }
    pub async fn request(
        &self,
        agent: String,
        method: String,
        path: String,
        headers: String,
        body: Vec<u8>,
    ) -> Result<String, JsError> {
        let port = 8642;
        if method != "GET" || !path.starts_with("/api/") || path.contains(['\r', '\n', '#']) {
            return Err(err("Only agent API GET requests are supported"));
        }
        let (io, connection) = self.tunnel(&agent, port).await?;
        let (mut sender, driver) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
                .await
                .map_err(err)?;
        wasm_bindgen_futures::spawn_local(async move {
            let _keep = connection.clone();
            let _ = driver.await;
        });
        let mut request = hyper::Request::builder()
            .method(method.as_str())
            .uri(path)
            .header("Host", format!("127.0.0.1:{port}"));
        let headers: BTreeMap<String, String> = serde_json::from_str(&headers).map_err(err)?;
        for (k, v) in headers {
            request = request.header(k, v);
        }
        let response = sender
            .send_request(request.body(Full::new(Bytes::from(body))).map_err(err)?)
            .await
            .map_err(err)?;
        let status = response.status().as_u16();
        let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in response.headers() {
            headers
                .entry(k.to_string())
                .or_default()
                .push(v.to_str().map_err(err)?.to_string());
        }
        let mut incoming = response.into_body();
        let mut data = Vec::new();
        while let Some(frame) = incoming.frame().await {
            if let Ok(chunk) = frame.map_err(err)?.into_data() {
                if data.len() + chunk.len() > 1024 * 1024 {
                    return Err(err("HTTP response exceeds 1 MiB"));
                }
                data.extend_from_slice(&chunk);
            }
        }
        serde_json::to_string(&serde_json::json!({"status":status,"headers":headers,"body":data}))
            .map_err(err)
    }
}
impl BrowserPeer {
    async fn tunnel(&self, agent: &str, port: u16) -> Result<(Io, Connection), JsError> {
        let id: EndpointId = agent.parse().map_err(err)?;
        let addr = EndpointAddr::new(id).with_relay_url(self.relay.clone());
        let c = self
            .endpoint
            .connect(addr, b"iroh-http-proxy/1")
            .await
            .map_err(err)?;
        let (mut send, mut recv) = c.open_bi().await.map_err(err)?;
        send.write_all(
            format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .map_err(err)?;
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let b = recv.read_u8().await.map_err(err)?;
            header.push(b);
            if header.len() > 8192 {
                return Err(err("CONNECT header limit"));
            }
        }
        if !header.starts_with(b"HTTP/1.1 200") {
            return Err(err(format!(
                "CONNECT rejected: {}",
                String::from_utf8_lossy(&header)
            )));
        }
        Ok((tokio::io::join(recv, send), c))
    }
}
