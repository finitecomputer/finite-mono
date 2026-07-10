use std::time::Duration;

use finitechat_proto::{
    RuntimeCommandDeliveryAckV1, RuntimeCommandDeliveryV1, RuntimeCommandResultDeliveryV1,
    RuntimeStateSnapshotDeliveryV1,
};
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::AgentdError;

const MAX_STREAM_LINE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct BridgeClient {
    base_url: String,
    client: Client,
}

impl BridgeClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, AgentdError> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        if !base_url.starts_with("http://127.0.0.1:") && !base_url.starts_with("http://localhost:")
        {
            return Err(AgentdError::Transport(
                "Finite Chat bridge must use a loopback URL".to_owned(),
            ));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self { base_url, client })
    }

    pub async fn wait_until_ready(&self) -> Result<(), AgentdError> {
        let response = self
            .client
            .get(format!("{}/readyz", self.base_url))
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(AgentdError::Transport(format!(
                "Finite Chat bridge is not ready ({})",
                response.status()
            )));
        }
        Ok(())
    }

    pub async fn stream_deliveries(
        &self,
        tx: mpsc::Sender<RuntimeCommandDeliveryV1>,
    ) -> Result<(), AgentdError> {
        let response = self
            .client
            .get(format!("{}/v1/agentd/inbound", self.base_url))
            .query(&[("limit", "64"), ("timeout_millis", "30000")])
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(AgentdError::Transport(format!(
                "command stream failed ({})",
                response.status()
            )));
        }
        let mut stream = response.bytes_stream();
        let mut buffered = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffered.extend_from_slice(&chunk);
            while let Some(index) = buffered.iter().position(|byte| *byte == b'\n') {
                if index > MAX_STREAM_LINE_BYTES {
                    return Err(AgentdError::Transport(
                        "command stream record exceeded the bounded buffer".to_owned(),
                    ));
                }
                let line = buffered.drain(..=index).collect::<Vec<_>>();
                let line = &line[..line.len().saturating_sub(1)];
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let delivery = serde_json::from_slice::<RuntimeCommandDeliveryV1>(line)?;
                delivery
                    .validate_structure()
                    .map_err(|error| AgentdError::Transport(error.to_string()))?;
                if tx.send(delivery).await.is_err() {
                    return Ok(());
                }
            }
            if buffered.len() > MAX_STREAM_LINE_BYTES {
                return Err(AgentdError::Transport(
                    "command stream record exceeded the bounded buffer".to_owned(),
                ));
            }
        }
        Err(AgentdError::Transport(
            "command stream ended unexpectedly".to_owned(),
        ))
    }

    pub async fn acknowledge(
        &self,
        delivery: &RuntimeCommandDeliveryV1,
    ) -> Result<(), AgentdError> {
        let ack = RuntimeCommandDeliveryAckV1 {
            room_id: delivery.room_id.clone(),
            seq: delivery.seq,
            message_id: delivery.message_id.clone(),
        };
        self.post("/v1/agentd/ack", &ack).await.map(|_| ())
    }

    pub async fn send_result(
        &self,
        delivery: RuntimeCommandResultDeliveryV1,
    ) -> Result<(), AgentdError> {
        self.post_allow_exact_replay("/v1/agentd/result", &delivery)
            .await
            .map(|_| ())
    }

    pub async fn send_state(
        &self,
        delivery: RuntimeStateSnapshotDeliveryV1,
    ) -> Result<(), AgentdError> {
        self.post("/v1/agentd/state", &delivery).await.map(|_| ())
    }

    pub async fn recover_chat(&self) -> Result<Value, AgentdError> {
        self.post("/v1/hermes/recover", &serde_json::json!({}))
            .await
    }

    async fn post(
        &self,
        path: &str,
        payload: &impl serde::Serialize,
    ) -> Result<Value, AgentdError> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .timeout(Duration::from_secs(30))
            .json(payload)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(AgentdError::Transport(format!(
                "resident bridge rejected {path} ({status})"
            )));
        }
        Ok(response.json().await?)
    }

    async fn post_allow_exact_replay(
        &self,
        path: &str,
        payload: &impl serde::Serialize,
    ) -> Result<Value, AgentdError> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .timeout(Duration::from_secs(30))
            .json(payload)
            .send()
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::CONFLICT {
            // agentd's durable command ledger replays byte-identical terminal
            // results for a request id. A conflict therefore means the room
            // already accepted this exact terminal result before an ack or
            // connection failure; continuing is the recoverable path.
            return Ok(serde_json::json!({ "replayed": true }));
        }
        if !status.is_success() {
            return Err(AgentdError::Transport(format!(
                "resident bridge rejected {path} ({status})"
            )));
        }
        Ok(response.json().await?)
    }
}
