use finitechat_proto::{
    ActivityId, ActivityKind, AttachmentBlobReferenceV1, ConversationId, ConversationSegmentId,
    EphemeralActivityActionV1, FINITECHAT_ACTIVITY_KIND_WORKING, MAX_ATTACHMENT_BLOB_URL_BYTES,
    MAX_ATTACHMENT_FILENAME_BYTES, MAX_ATTACHMENT_MIME_TYPE_BYTES, MAX_ENVELOPE_PAYLOAD_BYTES,
    MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES, MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
    MAX_OBJECT_ID_BYTES, MAX_SYNC_PAGE_ENTRIES, MessageId, ProtocolLimitError, RoomId, Seq,
    validate_bytes_len, validate_bytes_non_empty, validate_item_count, validate_room_id,
    validate_string_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

pub const FINITECHAT_HERMES_PLATFORM_NAME: &str = "finitechat";
pub const HERMES_METADATA_THREAD_ID: &str = "thread_id";
pub const HERMES_METADATA_CONVERSATION_ID: &str = "conversation_id";
pub const HERMES_METADATA_ATTACHMENTS: &str = "attachments";
pub const HERMES_METADATA_KIND: &str = "_finitechat_kind";
pub const HERMES_METADATA_STATUS: &str = "_finitechat_status";
pub const MAX_HERMES_POLL_EVENTS: u32 = 32;
pub const MAX_HERMES_TEXT_BYTES: u32 = 64 * 1024;
pub const MAX_HERMES_ATTACHMENTS: u32 = 16;
pub const MAX_HERMES_METADATA_BYTES: u32 = 32 * 1024;
pub const MAX_HERMES_REPLY_TEXT_BYTES: u32 = 8 * 1024;
pub const MAX_HERMES_POLL_TIMEOUT_MILLIS: u64 = 60 * 1000;

const _: () = {
    assert!(MAX_HERMES_POLL_EVENTS > 0);
    assert!(MAX_HERMES_POLL_EVENTS <= MAX_SYNC_PAGE_ENTRIES);
    assert!(MAX_HERMES_TEXT_BYTES > 0);
    assert!(MAX_HERMES_TEXT_BYTES < MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_HERMES_ATTACHMENTS > 0);
    assert!(MAX_HERMES_METADATA_BYTES > 0);
    assert!(MAX_HERMES_METADATA_BYTES < MAX_ENVELOPE_PAYLOAD_BYTES);
    assert!(MAX_HERMES_REPLY_TEXT_BYTES > 0);
    assert!(MAX_HERMES_REPLY_TEXT_BYTES < MAX_HERMES_TEXT_BYTES);
    assert!(MAX_HERMES_POLL_TIMEOUT_MILLIS > 0);
};

#[derive(Debug, Error)]
pub enum HermesBridgeError {
    #[error("missing required field {field}")]
    MissingField { field: String },
    #[error("metadata field {field} must be a string")]
    MetadataString { field: String },
    #[error("metadata field {field} has unknown value {value}")]
    UnknownMetadataValue { field: String, value: String },
    #[error("poll limit must be between 1 and {max}")]
    InvalidPollLimit { max: u32 },
    #[error("poll event source chat_id {source_chat_id} does not match room_id {room_id}")]
    SourceRoomMismatch {
        room_id: RoomId,
        source_chat_id: RoomId,
    },
    #[error(
        "poll event source thread_id {source_thread_id:?} does not match conversation_id {conversation_id:?}"
    )]
    SourceThreadMismatch {
        conversation_id: Option<ConversationId>,
        source_thread_id: Option<ConversationId>,
    },
    #[error(transparent)]
    Protocol(#[from] ProtocolLimitError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HermesMessageTypeV1 {
    Text,
    Location,
    Photo,
    Video,
    Audio,
    Voice,
    Document,
    Sticker,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HermesChatTypeV1 {
    Dm,
    Group,
    Channel,
    Thread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HermesAttachmentKindV1 {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HermesSendKindV1 {
    Message,
    Status,
    Tool,
    Media,
}

impl HermesSendKindV1 {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "message" => Some(Self::Message),
            "status" => Some(Self::Status),
            "tool" => Some(Self::Tool),
            "media" => Some(Self::Media),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HermesMessageStatusV1 {
    Running,
    Complete,
}

impl HermesMessageStatusV1 {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "complete" => Some(Self::Complete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesAttachmentV1 {
    pub kind: HermesAttachmentKindV1,
    pub name: String,
    pub mime_type: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub blob: Option<AttachmentBlobReferenceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesSourceV1 {
    pub platform: String,
    pub chat_id: RoomId,
    pub chat_name: Option<String>,
    pub chat_type: HermesChatTypeV1,
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub thread_id: Option<ConversationId>,
    pub chat_topic: Option<String>,
    pub user_id_alt: Option<String>,
    pub chat_id_alt: Option<String>,
    pub is_bot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesPollEventV1 {
    pub room_id: RoomId,
    pub seq: Seq,
    pub message_id: MessageId,
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<ConversationSegmentId>,
    pub text: String,
    pub message_type: HermesMessageTypeV1,
    pub source: HermesSourceV1,
    #[serde(default)]
    pub attachments: Vec<HermesAttachmentV1>,
    pub reply_to_message_id: Option<MessageId>,
    pub reply_to_text: Option<String>,
    pub auto_skill: Option<String>,
    pub channel_prompt: Option<String>,
    #[serde(default)]
    pub internal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesPollOptionsV1 {
    pub room_id: RoomId,
    pub after_seq: Option<Seq>,
    pub limit: u32,
    pub timeout_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesPollResponseV1 {
    pub events: Vec<HermesPollEventV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesAckRequestV1 {
    pub room_id: RoomId,
    pub seq: Seq,
    pub message_id: MessageId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesSendRequestV1 {
    pub room_id: RoomId,
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<ConversationSegmentId>,
    pub text: String,
    pub kind: HermesSendKindV1,
    pub status: HermesMessageStatusV1,
    #[serde(default)]
    pub attachments: Vec<HermesAttachmentV1>,
    pub reply_to_message_id: Option<MessageId>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermesSendResponseV1 {
    pub message_id: MessageId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesEditRequestV1 {
    pub room_id: RoomId,
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<ConversationSegmentId>,
    pub message_id: MessageId,
    pub text: String,
    pub status: HermesMessageStatusV1,
    pub finalize: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesActivityRequestV1 {
    pub room_id: RoomId,
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<ConversationSegmentId>,
    pub activity_kind: ActivityKind,
    pub activity_id: Option<ActivityId>,
    pub action: EphemeralActivityActionV1,
    #[serde(default)]
    pub payload: Value,
    pub expires_in_millis: u64,
}

impl HermesAttachmentV1 {
    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_bytes_non_empty("hermes.attachment.name", self.name.len())?;
        validate_string_bytes(
            "hermes.attachment.name",
            &self.name,
            MAX_ATTACHMENT_FILENAME_BYTES,
        )?;
        validate_bytes_non_empty("hermes.attachment.mime_type", self.mime_type.len())?;
        validate_string_bytes(
            "hermes.attachment.mime_type",
            &self.mime_type,
            MAX_ATTACHMENT_MIME_TYPE_BYTES,
        )?;
        validate_optional_string(
            "hermes.attachment.path",
            self.path.as_deref(),
            MAX_ATTACHMENT_BLOB_URL_BYTES,
        )?;
        validate_optional_string(
            "hermes.attachment.url",
            self.url.as_deref(),
            MAX_ATTACHMENT_BLOB_URL_BYTES,
        )?;
        if let Some(blob) = &self.blob {
            blob.validate_limits()?;
        }
        if self.path.is_none() && self.url.is_none() && self.blob.is_none() {
            return Err(HermesBridgeError::MissingField {
                field: "hermes.attachment.path_or_url_or_blob".to_string(),
            });
        }
        Ok(())
    }

    pub fn message_type(&self) -> HermesMessageTypeV1 {
        match self.kind {
            HermesAttachmentKindV1::Image => HermesMessageTypeV1::Photo,
            HermesAttachmentKindV1::Video => HermesMessageTypeV1::Video,
            HermesAttachmentKindV1::Audio => HermesMessageTypeV1::Audio,
            HermesAttachmentKindV1::File => HermesMessageTypeV1::Document,
        }
    }
}

impl HermesSourceV1 {
    pub fn finite_chat(
        room_id: impl Into<RoomId>,
        conversation_id: Option<impl Into<ConversationId>>,
        user_id: Option<impl Into<String>>,
        user_name: Option<impl Into<String>>,
    ) -> Result<Self, HermesBridgeError> {
        let source = Self {
            platform: FINITECHAT_HERMES_PLATFORM_NAME.to_string(),
            chat_id: room_id.into(),
            chat_name: None,
            chat_type: HermesChatTypeV1::Dm,
            user_id: user_id.map(Into::into),
            user_name: user_name.map(Into::into),
            thread_id: conversation_id.map(Into::into),
            chat_topic: None,
            user_id_alt: None,
            chat_id_alt: None,
            is_bot: false,
        };
        source.validate_limits()?;
        Ok(source)
    }

    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_bytes_non_empty("hermes.source.platform", self.platform.len())?;
        validate_string_bytes(
            "hermes.source.platform",
            &self.platform,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_hermes_room_id(&self.chat_id)?;
        validate_optional_string(
            "hermes.source.chat_name",
            self.chat_name.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.source.user_id",
            self.user_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.source.user_name",
            self.user_name.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.source.thread_id",
            self.thread_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.source.chat_topic",
            self.chat_topic.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.source.user_id_alt",
            self.user_id_alt.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.source.chat_id_alt",
            self.chat_id_alt.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        Ok(())
    }
}

impl HermesPollEventV1 {
    pub fn finite_chat_text(
        room_id: impl Into<RoomId>,
        seq: Seq,
        message_id: impl Into<MessageId>,
        sender_account_id: impl Into<String>,
        sender_device_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Self, HermesBridgeError> {
        let room_id = room_id.into();
        let event = Self {
            room_id: room_id.clone(),
            seq,
            message_id: message_id.into(),
            conversation_id: None,
            segment_id: None,
            text: text.into(),
            message_type: HermesMessageTypeV1::Text,
            source: HermesSourceV1 {
                platform: FINITECHAT_HERMES_PLATFORM_NAME.to_owned(),
                chat_id: room_id,
                chat_name: None,
                chat_type: HermesChatTypeV1::Group,
                user_id: Some(sender_account_id.into()),
                user_name: None,
                thread_id: None,
                chat_topic: None,
                user_id_alt: Some(sender_device_id.into()),
                chat_id_alt: None,
                is_bot: false,
            },
            attachments: Vec::new(),
            reply_to_message_id: None,
            reply_to_text: None,
            auto_skill: None,
            channel_prompt: None,
            internal: false,
        };
        event.validate_limits()?;
        Ok(event)
    }

    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_hermes_room_id(&self.room_id)?;
        validate_message_id(&self.message_id)?;
        validate_optional_string(
            "hermes.poll_event.conversation_id",
            self.conversation_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.poll_event.segment_id",
            self.segment_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_string_bytes("hermes.poll_event.text", &self.text, MAX_HERMES_TEXT_BYTES)?;
        self.source.validate_limits()?;
        if self.source.chat_id != self.room_id {
            return Err(HermesBridgeError::SourceRoomMismatch {
                room_id: self.room_id.clone(),
                source_chat_id: self.source.chat_id.clone(),
            });
        }
        if self.source.thread_id != self.conversation_id {
            return Err(HermesBridgeError::SourceThreadMismatch {
                conversation_id: self.conversation_id.clone(),
                source_thread_id: self.source.thread_id.clone(),
            });
        }
        validate_attachments(&self.attachments)?;
        validate_optional_string(
            "hermes.poll_event.reply_to_message_id",
            self.reply_to_message_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.poll_event.reply_to_text",
            self.reply_to_text.as_deref(),
            MAX_HERMES_REPLY_TEXT_BYTES,
        )?;
        validate_optional_string(
            "hermes.poll_event.auto_skill",
            self.auto_skill.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.poll_event.channel_prompt",
            self.channel_prompt.as_deref(),
            MAX_HERMES_METADATA_BYTES,
        )?;
        Ok(())
    }
}

impl HermesPollOptionsV1 {
    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_hermes_room_id(&self.room_id)?;
        if self.limit == 0 || self.limit > MAX_HERMES_POLL_EVENTS {
            return Err(HermesBridgeError::InvalidPollLimit {
                max: MAX_HERMES_POLL_EVENTS,
            });
        }
        if self.timeout_millis > MAX_HERMES_POLL_TIMEOUT_MILLIS {
            return Err(ProtocolLimitError::BytesTooLong {
                field: "hermes.poll.timeout_millis".to_string(),
                max_bytes: MAX_HERMES_POLL_TIMEOUT_MILLIS,
                actual_bytes: self.timeout_millis,
            }
            .into());
        }
        Ok(())
    }
}

impl HermesPollResponseV1 {
    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_item_count(
            "hermes.poll.events",
            self.events.len(),
            MAX_HERMES_POLL_EVENTS,
        )?;
        for event in &self.events {
            event.validate_limits()?;
        }
        Ok(())
    }
}

impl HermesAckRequestV1 {
    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_hermes_room_id(&self.room_id)?;
        validate_message_id(&self.message_id)?;
        Ok(())
    }
}

impl HermesSendRequestV1 {
    pub fn from_hermes_send(
        chat_id: impl Into<RoomId>,
        content: impl Into<String>,
        reply_to_message_id: Option<impl Into<MessageId>>,
        mut metadata: BTreeMap<String, Value>,
    ) -> Result<Self, HermesBridgeError> {
        let conversation_id = match take_string_metadata(&mut metadata, HERMES_METADATA_THREAD_ID)?
        {
            Some(conversation_id) => Some(conversation_id),
            None => take_string_metadata(&mut metadata, HERMES_METADATA_CONVERSATION_ID)?,
        };
        let kind = take_send_kind(&mut metadata)?.unwrap_or(HermesSendKindV1::Message);
        let status = take_message_status(&mut metadata)?.unwrap_or(HermesMessageStatusV1::Complete);
        let attachments = take_attachments(&mut metadata)?;
        let kind = if attachments.is_empty() {
            kind
        } else {
            HermesSendKindV1::Media
        };
        let request = Self {
            room_id: chat_id.into(),
            conversation_id,
            segment_id: None,
            text: content.into(),
            kind,
            status,
            attachments,
            reply_to_message_id: reply_to_message_id.map(Into::into),
            metadata,
        };
        request.validate_limits()?;
        Ok(request)
    }

    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_hermes_room_id(&self.room_id)?;
        validate_optional_string(
            "hermes.send.conversation_id",
            self.conversation_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.send.segment_id",
            self.segment_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_string_bytes("hermes.send.text", &self.text, MAX_HERMES_TEXT_BYTES)?;
        validate_attachments(&self.attachments)?;
        validate_optional_string(
            "hermes.send.reply_to_message_id",
            self.reply_to_message_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_json_value_bytes(
            "hermes.send.metadata",
            &self.metadata,
            MAX_HERMES_METADATA_BYTES,
        )?;
        Ok(())
    }
}

impl HermesSendResponseV1 {
    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_message_id(&self.message_id)?;
        Ok(())
    }
}

impl HermesEditRequestV1 {
    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_hermes_room_id(&self.room_id)?;
        validate_optional_string(
            "hermes.edit.conversation_id",
            self.conversation_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.edit.segment_id",
            self.segment_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_message_id(&self.message_id)?;
        validate_string_bytes("hermes.edit.text", &self.text, MAX_HERMES_TEXT_BYTES)?;
        validate_json_value_bytes(
            "hermes.edit.metadata",
            &self.metadata,
            MAX_HERMES_METADATA_BYTES,
        )?;
        Ok(())
    }
}

impl HermesActivityRequestV1 {
    pub fn working(
        room_id: impl Into<RoomId>,
        conversation_id: Option<impl Into<ConversationId>>,
        action: EphemeralActivityActionV1,
    ) -> Result<Self, HermesBridgeError> {
        let request = Self {
            room_id: room_id.into(),
            conversation_id: conversation_id.map(Into::into),
            segment_id: None,
            activity_kind: FINITECHAT_ACTIVITY_KIND_WORKING.to_string(),
            activity_id: None,
            action,
            payload: Value::Null,
            expires_in_millis: MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS.min(5 * 60 * 1000),
        };
        request.validate_limits()?;
        Ok(request)
    }

    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_hermes_room_id(&self.room_id)?;
        validate_optional_string(
            "hermes.activity.conversation_id",
            self.conversation_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.activity.segment_id",
            self.segment_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_bytes_non_empty("hermes.activity.kind", self.activity_kind.len())?;
        validate_string_bytes(
            "hermes.activity.kind",
            &self.activity_kind,
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_optional_string(
            "hermes.activity.activity_id",
            self.activity_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )?;
        validate_json_value_bytes(
            "hermes.activity.payload",
            &self.payload,
            MAX_EPHEMERAL_ACTIVITY_DECRYPTED_PAYLOAD_BYTES,
        )?;
        if self.expires_in_millis == 0
            || self.expires_in_millis > MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS
        {
            return Err(ProtocolLimitError::BytesTooLong {
                field: "hermes.activity.expires_in_millis".to_string(),
                max_bytes: MAX_EPHEMERAL_ACTIVITY_EXPIRY_MILLIS,
                actual_bytes: self.expires_in_millis,
            }
            .into());
        }
        Ok(())
    }
}

fn validate_attachments(attachments: &[HermesAttachmentV1]) -> Result<(), HermesBridgeError> {
    validate_item_count(
        "hermes.attachments",
        attachments.len(),
        MAX_HERMES_ATTACHMENTS,
    )?;
    for attachment in attachments {
        attachment.validate_limits()?;
    }
    Ok(())
}

fn validate_message_id(message_id: &str) -> Result<(), ProtocolLimitError> {
    validate_bytes_non_empty("message_id", message_id.len())?;
    validate_string_bytes("message_id", message_id, MAX_OBJECT_ID_BYTES)
}

fn validate_hermes_room_id(room_id: &str) -> Result<(), ProtocolLimitError> {
    validate_bytes_non_empty("room_id", room_id.len())?;
    validate_room_id(room_id)
}

fn validate_optional_string(
    field: &str,
    value: Option<&str>,
    max_bytes: u32,
) -> Result<(), ProtocolLimitError> {
    if let Some(value) = value {
        validate_bytes_non_empty(field, value.len())?;
        validate_string_bytes(field, value, max_bytes)?;
    }
    Ok(())
}

fn validate_json_value_bytes<T: Serialize>(
    field: &str,
    value: &T,
    max_bytes: u32,
) -> Result<(), HermesBridgeError> {
    let bytes = serde_json::to_vec(value)?;
    validate_bytes_len(field, bytes.len(), max_bytes)?;
    Ok(())
}

fn take_string_metadata(
    metadata: &mut BTreeMap<String, Value>,
    key: &str,
) -> Result<Option<String>, HermesBridgeError> {
    let Some(value) = metadata.remove(key) else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(value) if value.is_empty() => Ok(None),
        Value::String(value) => {
            validate_string_bytes(key, &value, MAX_OBJECT_ID_BYTES)?;
            Ok(Some(value))
        }
        _ => Err(HermesBridgeError::MetadataString {
            field: key.to_string(),
        }),
    }
}

fn take_send_kind(
    metadata: &mut BTreeMap<String, Value>,
) -> Result<Option<HermesSendKindV1>, HermesBridgeError> {
    let Some(value) = take_string_metadata(metadata, HERMES_METADATA_KIND)? else {
        return Ok(None);
    };
    HermesSendKindV1::parse(&value)
        .ok_or(HermesBridgeError::UnknownMetadataValue {
            field: HERMES_METADATA_KIND.to_string(),
            value,
        })
        .map(Some)
}

fn take_message_status(
    metadata: &mut BTreeMap<String, Value>,
) -> Result<Option<HermesMessageStatusV1>, HermesBridgeError> {
    let Some(value) = take_string_metadata(metadata, HERMES_METADATA_STATUS)? else {
        return Ok(None);
    };
    HermesMessageStatusV1::parse(&value)
        .ok_or(HermesBridgeError::UnknownMetadataValue {
            field: HERMES_METADATA_STATUS.to_string(),
            value,
        })
        .map(Some)
}

fn take_attachments(
    metadata: &mut BTreeMap<String, Value>,
) -> Result<Vec<HermesAttachmentV1>, HermesBridgeError> {
    let Some(value) = metadata.remove(HERMES_METADATA_ATTACHMENTS) else {
        return Ok(Vec::new());
    };
    let attachments = serde_json::from_value::<Vec<HermesAttachmentV1>>(value)?;
    validate_attachments(&attachments)?;
    Ok(attachments)
}

/// The decrypted application payload the Hermes bridge writes into a room
/// (ADR 0002: Rust owns the schema; Python stays a translator). Non-hermes
/// payloads in the same room are skipped by `decode`, so agents coexist
/// with other application traffic.
pub const HERMES_MESSAGE_PAYLOAD_TYPE_V1: &str = "finitechat.hermes.message.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesMessagePayloadV1 {
    #[serde(rename = "type")]
    pub payload_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<ConversationSegmentId>,
    pub text: String,
    pub kind: HermesSendKindV1,
    pub status: HermesMessageStatusV1,
    /// For edits: the message id of the entry being superseded. Edits are
    /// new log entries (the log is append-only); renderers show the latest
    /// payload for an `edit_of` chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_of: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<HermesAttachmentV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl HermesMessagePayloadV1 {
    pub fn from_send(request: &HermesSendRequestV1) -> Self {
        Self {
            payload_type: HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: request.conversation_id.clone(),
            segment_id: request.segment_id.clone(),
            text: request.text.clone(),
            kind: request.kind,
            status: request.status,
            edit_of: None,
            attachments: request.attachments.clone(),
            reply_to_message_id: request.reply_to_message_id.clone(),
            sender_name: None,
            metadata: request.metadata.clone(),
        }
    }

    pub fn from_edit(request: &HermesEditRequestV1) -> Self {
        Self {
            payload_type: HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: request.conversation_id.clone(),
            segment_id: request.segment_id.clone(),
            text: request.text.clone(),
            kind: HermesSendKindV1::Message,
            status: request.status,
            edit_of: Some(request.message_id.clone()),
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: None,
            metadata: request.metadata.clone(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, HermesBridgeError> {
        self.validate_limits()?;
        Ok(serde_json::to_vec(self)?)
    }

    /// `Ok(None)` when the plaintext is not a hermes message payload —
    /// other application traffic in the room is simply not bridge-visible.
    pub fn decode(plaintext: &[u8]) -> Result<Option<Self>, HermesBridgeError> {
        let Ok(value) = serde_json::from_slice::<Value>(plaintext) else {
            return Ok(None);
        };
        if value.get("type").and_then(Value::as_str) != Some(HERMES_MESSAGE_PAYLOAD_TYPE_V1) {
            return Ok(None);
        }
        let payload: Self = serde_json::from_value(value)?;
        payload.validate_limits()?;
        Ok(Some(payload))
    }

    pub fn validate_limits(&self) -> Result<(), HermesBridgeError> {
        validate_string_bytes("hermes.payload.text", &self.text, MAX_HERMES_TEXT_BYTES)
            .map_err(HermesBridgeError::from)?;
        validate_optional_string(
            "hermes.payload.conversation_id",
            self.conversation_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(HermesBridgeError::from)?;
        validate_optional_string(
            "hermes.payload.segment_id",
            self.segment_id.as_deref(),
            MAX_OBJECT_ID_BYTES,
        )
        .map_err(HermesBridgeError::from)?;
        validate_item_count(
            "hermes.payload.attachments",
            self.attachments.len(),
            MAX_HERMES_ATTACHMENTS,
        )
        .map_err(HermesBridgeError::from)?;
        for attachment in &self.attachments {
            attachment.validate_limits()?;
        }
        Ok(())
    }

    /// Build the poll event Hermes consumes. `sender_account_id` and
    /// `sender_device_id` come from the MLS-authenticated decryption
    /// result, never from the payload.
    pub fn into_poll_event(
        self,
        room_id: impl Into<RoomId>,
        seq: Seq,
        message_id: impl Into<MessageId>,
        sender_account_id: impl Into<String>,
        sender_device_id: impl Into<String>,
    ) -> HermesPollEventV1 {
        let room_id = room_id.into();
        let sender_account_id = sender_account_id.into();
        let message_type = self
            .attachments
            .first()
            .map(HermesAttachmentV1::message_type)
            .unwrap_or(HermesMessageTypeV1::Text);
        HermesPollEventV1 {
            room_id: room_id.clone(),
            seq,
            message_id: message_id.into(),
            conversation_id: self.conversation_id.clone(),
            segment_id: self.segment_id.clone(),
            text: self.text,
            message_type,
            source: HermesSourceV1 {
                platform: FINITECHAT_HERMES_PLATFORM_NAME.to_owned(),
                chat_id: room_id,
                chat_name: None,
                chat_type: HermesChatTypeV1::Group,
                user_id: Some(sender_account_id),
                user_name: self.sender_name,
                thread_id: self.segment_id.clone().or(self.conversation_id.clone()),
                chat_topic: None,
                user_id_alt: Some(sender_device_id.into()),
                chat_id_alt: None,
                is_bot: false,
            },
            attachments: self.attachments,
            reply_to_message_id: self.reply_to_message_id,
            reply_to_text: None,
            auto_skill: None,
            channel_prompt: None,
            internal: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_maps_room_and_conversation_to_hermes_chat_and_thread() {
        let source = HermesSourceV1::finite_chat(
            "room-agent-1",
            Some("topic-build"),
            Some("alice"),
            Some("Alice"),
        )
        .expect("source is valid");

        assert_eq!(source.platform, FINITECHAT_HERMES_PLATFORM_NAME);
        assert_eq!(source.chat_id, "room-agent-1");
        assert_eq!(source.thread_id.as_deref(), Some("topic-build"));
        assert_eq!(source.chat_type, HermesChatTypeV1::Dm);
        assert_eq!(source.user_id.as_deref(), Some("alice"));
    }

    #[test]
    fn poll_event_rejects_source_room_mismatch() {
        let mut event = sample_poll_event();
        event.source.chat_id = "other-room".to_string();

        let error = event
            .validate_limits()
            .expect_err("room mismatch is rejected");
        assert!(matches!(
            error,
            HermesBridgeError::SourceRoomMismatch { .. }
        ));
    }

    #[test]
    fn poll_response_rejects_too_many_events() {
        let mut events = Vec::new();
        for index in 0..=MAX_HERMES_POLL_EVENTS {
            let mut event = sample_poll_event();
            event.seq = Seq::from(index);
            event.message_id = format!("message-{index}");
            events.push(event);
        }
        assert_eq!(events.len(), MAX_HERMES_POLL_EVENTS as usize + 1);

        let response = HermesPollResponseV1 { events };
        assert!(response.validate_limits().is_err());
    }

    #[test]
    fn send_request_extracts_conversation_private_fields_and_attachments() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            HERMES_METADATA_THREAD_ID.to_string(),
            Value::String("topic-build".to_string()),
        );
        metadata.insert(
            HERMES_METADATA_KIND.to_string(),
            Value::String("tool".to_string()),
        );
        metadata.insert(
            HERMES_METADATA_STATUS.to_string(),
            Value::String("running".to_string()),
        );
        metadata.insert("visible".to_string(), Value::Bool(true));
        metadata.insert(
            HERMES_METADATA_ATTACHMENTS.to_string(),
            serde_json::to_value(vec![sample_attachment()]).expect("attachment serializes"),
        );

        let request = HermesSendRequestV1::from_hermes_send(
            "room-agent-1",
            "build log",
            Some("m-1"),
            metadata,
        )
        .expect("send request is valid");

        assert_eq!(request.room_id, "room-agent-1");
        assert_eq!(request.conversation_id.as_deref(), Some("topic-build"));
        assert_eq!(request.kind, HermesSendKindV1::Media);
        assert_eq!(request.status, HermesMessageStatusV1::Running);
        assert_eq!(request.reply_to_message_id.as_deref(), Some("m-1"));
        assert_eq!(request.attachments.len(), 1);
        assert_eq!(request.metadata.get("visible"), Some(&Value::Bool(true)));
        assert!(!request.metadata.contains_key(HERMES_METADATA_THREAD_ID));
        assert!(!request.metadata.contains_key(HERMES_METADATA_KIND));
        assert!(!request.metadata.contains_key(HERMES_METADATA_STATUS));
    }

    #[test]
    fn send_request_accepts_status_kind() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            HERMES_METADATA_KIND.to_string(),
            Value::String("status".to_string()),
        );

        let request = HermesSendRequestV1::from_hermes_send(
            "room-agent-1",
            "Hermes is working",
            None::<String>,
            metadata,
        )
        .expect("status kind is valid");

        assert_eq!(request.kind, HermesSendKindV1::Status);
    }

    #[test]
    fn send_request_rejects_invalid_metadata_types() {
        let mut metadata = BTreeMap::new();
        metadata.insert(HERMES_METADATA_THREAD_ID.to_string(), Value::Bool(true));

        let error = HermesSendRequestV1::from_hermes_send(
            "room-agent-1",
            "hello",
            None::<String>,
            metadata,
        )
        .expect_err("non-string thread id is rejected");

        assert!(matches!(error, HermesBridgeError::MetadataString { .. }));
    }

    #[test]
    fn send_request_rejects_empty_room() {
        let error =
            HermesSendRequestV1::from_hermes_send("", "hello", None::<String>, BTreeMap::new())
                .expect_err("empty room is rejected");

        assert!(matches!(error, HermesBridgeError::Protocol(_)));
    }

    #[test]
    fn activity_request_supports_long_running_work_without_notification() {
        let set = HermesActivityRequestV1::working(
            "room-agent-1",
            Some("topic-build"),
            EphemeralActivityActionV1::Set,
        )
        .expect("working activity is valid");
        let clear = HermesActivityRequestV1::working(
            "room-agent-1",
            Some("topic-build"),
            EphemeralActivityActionV1::Clear,
        )
        .expect("working clear is valid");

        assert_eq!(set.activity_kind, FINITECHAT_ACTIVITY_KIND_WORKING);
        assert_eq!(set.expires_in_millis, 5 * 60 * 1000);
        assert_eq!(clear.action, EphemeralActivityActionV1::Clear);
    }

    #[test]
    fn poll_event_round_trips_through_json() {
        let event = sample_poll_event();
        let bytes = serde_json::to_vec(&event).expect("event serializes");
        assert!(bytes.len() < MAX_HERMES_METADATA_BYTES as usize);

        let decoded: HermesPollEventV1 = serde_json::from_slice(&bytes).expect("event decodes");
        decoded.validate_limits().expect("decoded event is valid");
        assert_eq!(decoded, event);
    }

    #[test]
    fn finite_chat_text_builds_authenticated_plain_message_event() {
        let event = HermesPollEventV1::finite_chat_text(
            "room-agent-1",
            8,
            "message-8",
            "alice-account",
            "ios",
            "hello from iOS",
        )
        .expect("plain text event is valid");

        assert_eq!(event.text, "hello from iOS");
        assert_eq!(event.message_type, HermesMessageTypeV1::Text);
        assert_eq!(event.source.chat_type, HermesChatTypeV1::Group);
        assert_eq!(event.source.user_id.as_deref(), Some("alice-account"));
        assert_eq!(event.source.user_id_alt.as_deref(), Some("ios"));
    }

    #[test]
    fn bounded_invalid_attachment_matrix_is_rejected() {
        let mut missing_ref = sample_attachment();
        missing_ref.path = None;
        missing_ref.url = None;
        missing_ref.blob = None;

        let mut missing_name = sample_attachment();
        missing_name.name.clear();

        let mut missing_mime = sample_attachment();
        missing_mime.mime_type.clear();

        let invalid = [missing_ref, missing_name, missing_mime];
        for attachment in invalid {
            assert!(attachment.validate_limits().is_err());
        }
    }

    fn sample_poll_event() -> HermesPollEventV1 {
        HermesPollEventV1 {
            room_id: "room-agent-1".to_string(),
            seq: 7,
            message_id: "message-7".to_string(),
            conversation_id: Some("topic-build".to_string()),
            segment_id: None,
            text: "hello".to_string(),
            message_type: HermesMessageTypeV1::Text,
            source: HermesSourceV1::finite_chat(
                "room-agent-1",
                Some("topic-build"),
                Some("alice"),
                Some("Alice"),
            )
            .expect("source is valid"),
            attachments: vec![sample_attachment()],
            reply_to_message_id: Some("message-6".to_string()),
            reply_to_text: Some("previous".to_string()),
            auto_skill: Some("coding".to_string()),
            channel_prompt: Some("project context".to_string()),
            internal: false,
        }
    }

    fn sample_attachment() -> HermesAttachmentV1 {
        HermesAttachmentV1 {
            kind: HermesAttachmentKindV1::Image,
            name: "screenshot.png".to_string(),
            mime_type: "image/png".to_string(),
            path: Some("/tmp/screenshot.png".to_string()),
            url: None,
            blob: None,
        }
    }
}
