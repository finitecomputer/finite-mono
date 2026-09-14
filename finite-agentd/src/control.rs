use crate::AgentdError;
use crate::config::{ConfigManager, HermesConfigOfferV1, HermesConfigRollbackV1};
use crate::connections::{
    ConnectionManager, GoogleApplyRequest, InferenceApplyRequest, PairingApproveRequest,
    TelegramConnectRequest, TelegramHomeRequest,
};
use crate::supervisor::SupervisorHandle;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

/// Agent-local operation. Contains no chat identity, room, or delivery envelope.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControlRequest {
    pub request_id: String,
    pub command: String,
    pub schema: String,
    pub body: Box<serde_json::value::RawValue>,
}

#[derive(Deserialize)]
struct EmptyRequest {}

#[derive(Clone)]
pub(crate) struct ConnectionControl {
    pub config_manager: ConfigManager,
    pub connection_manager: ConnectionManager,
    pub hermes_home: PathBuf,
    pub supervisor: SupervisorHandle,
    pub operation: std::sync::Arc<tokio::sync::Semaphore>,
}
const EMPTY_REQUEST_SCHEMA: &str = "finite.agent.empty.request.v1";
const INFERENCE_APPLY_SCHEMA: &str = "finite.agent.inference.apply.v1";
const TELEGRAM_CONNECT_SCHEMA: &str = "finite.agent.telegram.connect.v1";
const TELEGRAM_APPROVE_SCHEMA: &str = "finite.agent.telegram.approve.v1";
const TELEGRAM_HOME_SCHEMA: &str = "finite.agent.telegram.home.v1";
const GOOGLE_APPLY_SCHEMA: &str = "finite.agent.google.apply.v1";
impl ConnectionControl {
    pub(crate) async fn execute(&self, request: &ControlRequest) -> Result<Value, AgentdError> {
        match request.command.as_str() {
            "agent.connections.status" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let manager = self.connection_manager.clone();
                let status = tokio::task::spawn_blocking(move || manager.status())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(serde_json::to_value(status)?)
            }
            "agent.inference.apply" => {
                let body = parse_body::<InferenceApplyRequest>(request, INFERENCE_APPLY_SCHEMA)?;
                let plan = self
                    .connection_manager
                    .inference_plan(&request.request_id, body)?;
                self.apply_inference_plan(plan).await
            }
            "agent.simplex.connect" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .simplex_offer(&request.request_id, true)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.simplex_address())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                self.apply_config_offer(offer).await
            }
            "agent.simplex.reset" => {
                parse_body::<EmptyRequest>(request, "finite.agent.simplex.reset.v1")?;
                let offer = self
                    .connection_manager
                    .simplex_offer(&request.request_id, false)?;
                let config = self.config_manager.clone();
                let home = self.hermes_home.clone();
                let connections = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || {
                    config.apply(&offer, || validate_hermes_config(&home))?;
                    connections.prepare_simplex_reset()
                })
                .await
                .map_err(|error| AgentdError::Config(error.to_string()))??;
                // Even already-disabled connections need cleanup. The wrapper
                // completes durable reset intent before the next gateway starts.
                self.supervisor.restart_hermes().await?;
                let connections = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || connections.wait_simplex_reset())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))?
            }
            "agent.simplex.approve_request" => {
                let body = parse_body::<crate::connections::SimplexApproveRequest>(
                    request,
                    "finite.agent.simplex.approve-request.v1",
                )?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.approve_simplex_request(body))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "approved": true }))
            }
            "agent.telegram.connect" => {
                let body = parse_body::<TelegramConnectRequest>(request, TELEGRAM_CONNECT_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_connect_offer(&request.request_id, body)?;
                self.apply_config_offer(offer).await
            }
            "agent.telegram.approve" => {
                let body = parse_body::<PairingApproveRequest>(request, TELEGRAM_APPROVE_SCHEMA)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.approve_telegram(body))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "approved": true }))
            }
            "agent.telegram.home" => {
                let body = parse_body::<TelegramHomeRequest>(request, TELEGRAM_HOME_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_home_offer(&request.request_id, body)?;
                self.apply_config_offer(offer).await
            }
            "agent.telegram.disconnect" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let offer = self
                    .connection_manager
                    .telegram_disconnect_offer(&request.request_id)?;
                self.apply_config_offer(offer).await
            }
            "agent.google.apply" => {
                let body = parse_body::<GoogleApplyRequest>(request, GOOGLE_APPLY_SCHEMA)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.apply_google(body))
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "connected": true }))
            }
            "agent.google.disconnect" => {
                parse_body::<EmptyRequest>(request, EMPTY_REQUEST_SCHEMA)?;
                let manager = self.connection_manager.clone();
                tokio::task::spawn_blocking(move || manager.disconnect_google())
                    .await
                    .map_err(|error| AgentdError::Config(error.to_string()))??;
                Ok(json!({ "connected": false }))
            }
            command => Err(AgentdError::UnsupportedCommand(command.to_owned())),
        }
    }

    async fn apply_config_offer(&self, offer: HermesConfigOfferV1) -> Result<Value, AgentdError> {
        let manager = self.config_manager.clone();
        let hermes_home = self.hermes_home.clone();
        let result = tokio::task::spawn_blocking(move || {
            manager.apply(&offer, || validate_hermes_config(&hermes_home))
        })
        .await
        .map_err(|error| AgentdError::Config(error.to_string()))??;
        if result.restart_required {
            self.supervisor.restart_hermes().await?;
        }
        Ok(serde_json::to_value(result)?)
    }

    async fn apply_inference_plan(
        &self,
        plan: crate::connections::InferenceApplyPlan,
    ) -> Result<Value, AgentdError> {
        let credential_snapshot = self.connection_manager.stage_inference_credential(&plan)?;
        let proposal_id = plan.offer.proposal_id.clone();
        let manager = self.config_manager.clone();
        let hermes_home = self.hermes_home.clone();
        let apply = tokio::task::spawn_blocking(move || {
            manager.apply(&plan.offer, || validate_hermes_config(&hermes_home))
        })
        .await;
        let apply = match apply {
            Ok(apply) => apply,
            Err(error) => {
                self.connection_manager
                    .restore_inference_credential(credential_snapshot)
                    .map_err(|restore_error| {
                        AgentdError::Config(format!(
                            "inference apply task failed ({error}); previous credential could not be restored ({restore_error})"
                        ))
                    })?;
                return Err(AgentdError::Config(error.to_string()));
            }
        };
        let result = match apply {
            Ok(result) => result,
            Err(error) => {
                self.connection_manager
                    .restore_inference_credential(credential_snapshot)?;
                return Err(error);
            }
        };
        if !result.restart_required {
            return Ok(serde_json::to_value(result)?);
        }
        if let Err(restart_error) = self.supervisor.restart_hermes().await {
            let rollback = HermesConfigRollbackV1 { proposal_id };
            let manager = self.config_manager.clone();
            let hermes_home = self.hermes_home.clone();
            let rollback_result = tokio::task::spawn_blocking(move || {
                manager.rollback(&rollback, || validate_hermes_config(&hermes_home))
            })
            .await;
            let credential_restore = self
                .connection_manager
                .restore_inference_credential(credential_snapshot);
            let rollback_result = rollback_result.map_err(|rollback_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); configuration rollback task failed ({rollback_error})"
                ))
            })?;
            rollback_result.map_err(|rollback_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); previous configuration could not be restored ({rollback_error})"
                ))
            })?;
            credential_restore.map_err(|restore_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); previous credential could not be restored ({restore_error})"
                ))
            })?;
            self.supervisor.restart_hermes().await.map_err(|restore_error| {
                AgentdError::Supervisor(format!(
                    "Hermes inference activation failed ({restart_error}); previous configuration was restored but Hermes could not be reactivated ({restore_error})"
                ))
            })?;
            return Err(restart_error);
        }
        Ok(serde_json::to_value(result)?)
    }
}
fn parse_body<T: DeserializeOwned>(
    request: &ControlRequest,
    expected_schema: &str,
) -> Result<T, AgentdError> {
    if request.schema != expected_schema {
        return Err(AgentdError::InvalidPayload(format!(
            "expected schema {expected_schema:?}"
        )));
    }
    serde_json::from_str(request.body.get()).map_err(|_| {
        AgentdError::InvalidPayload("request JSON did not match its schema".to_owned())
    })
}
fn validate_hermes_config(hermes_home: &Path) -> Result<(), AgentdError> {
    let status = StdCommand::new("hermes")
        .arg("config")
        .arg("check")
        .env("HERMES_HOME", hermes_home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(AgentdError::Config(
            "Hermes rejected the proposed configuration; previous bytes were restored".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_extraction_preserves_duplicate_field_rejection() {
        let request: ControlRequest = serde_json::from_str(
            r#"{
            "request_id":"test", "command":"agent.telegram.home",
            "schema":"finite.agent.telegram.home.v1",
            "body":{"user_id":"123","user_id":"456"}
        }"#,
        )
        .unwrap();
        assert!(parse_body::<TelegramHomeRequest>(&request, TELEGRAM_HOME_SCHEMA).is_err());
    }
}
