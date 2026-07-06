use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CHAT_ATTACHMENT_BLOB_METADATA_KEY: &str = "finitechat_blob";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatUser {
    pub id: String,
    pub email: String,
    pub name: String,
    pub disabled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMachine {
    pub id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub kind: String,
    pub state: String,
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatProjectAgent {
    pub id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub machine_id: String,
    pub name: String,
    pub purpose: String,
    pub hermes_profile_name: String,
    pub hermes_profile_ref: String,
    pub hermes_api_port: u16,
    pub hermes_api_base_url: String,
    pub workspace_ref: String,
    pub google_bot_identity: Option<String>,
    pub replicated_from: Option<String>,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatThread {
    pub id: String,
    pub project_agent_id: String,
    pub created_by: String,
    pub title: String,
    pub created_at: String,
    pub last_activity_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub thread_id: String,
    pub sender_type: String,
    pub kind: String,
    pub status: String,
    pub body: String,
    #[serde(default)]
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessagePage {
    pub messages: Vec<ChatMessage>,
    pub has_more: bool,
    pub next_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatInboxEvent {
    pub seq: u64,
    pub kind: String,
    #[serde(rename = "projectAgentId")]
    pub project_agent_id: String,
    #[serde(rename = "conversationId")]
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ChatThread>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatInboxPage {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "projectAgentId")]
    pub project_agent_id: String,
    pub cursor: u64,
    pub events: Vec<ChatInboxEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessageAttachment {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub attachment_type: Option<String>,
    pub name: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub url: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessageAttachmentInput {
    pub name: String,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSite {
    pub id: String,
    pub project_agent_id: String,
    pub service_name: String,
    pub host: String,
    pub port: u16,
    pub mode: String,
    pub status: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSkill {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub capability_requirements: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatCapability {
    pub id: String,
    pub kind: String,
    pub display_name: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSlashCommand {
    pub name: String,
    pub description: String,
    pub category: String,
    pub aliases: Vec<String>,
    pub args_hint: String,
    pub subcommands: Vec<String>,
    pub cli_only: bool,
    pub gateway_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatBootstrapData {
    pub users: Vec<ChatUser>,
    pub machines: Vec<ChatMachine>,
    pub project_agents: Vec<ChatProjectAgent>,
    pub threads: Vec<ChatThread>,
    pub messages: Vec<ChatMessage>,
    pub sites: Vec<ChatSite>,
    pub skills: Vec<ChatSkill>,
    pub capabilities: Vec<ChatCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateChatThreadRequest {
    #[serde(rename = "threadId", default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendChatMessageRequest {
    pub body: String,
    pub client_message_id: Option<String>,
    #[serde(default)]
    pub attachments: Vec<ChatMessageAttachmentInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayPollRequest {
    pub agent_id: String,
    pub limit: Option<usize>,
    pub timeout_secs: Option<u64>,
}

impl GatewayPollRequest {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(10).clamp(1, 50)
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs.unwrap_or(20).min(60)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayPollResponse {
    pub events: Vec<GatewayMessageEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayMessageEvent {
    pub id: String,
    pub project_agent_id: String,
    pub thread_id: String,
    pub message_id: String,
    pub user_id: String,
    pub user_name: String,
    pub body: String,
    #[serde(default)]
    pub metadata: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayAckRequest {
    pub agent_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayOutboundMessageRequest {
    pub agent_id: String,
    pub thread_id: String,
    pub body: String,
    pub kind: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Value,
    pub reply_to_message_id: Option<String>,
    pub message_id: Option<String>,
}

impl GatewayOutboundMessageRequest {
    pub fn kind(&self) -> &str {
        self.kind.as_deref().unwrap_or("message")
    }

    pub fn status(&self) -> &str {
        self.status.as_deref().unwrap_or("complete")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayEditMessageRequest {
    pub agent_id: String,
    pub body: String,
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl GatewayEditMessageRequest {
    pub fn status(&self) -> &str {
        self.status.as_deref().unwrap_or("running")
    }
}
