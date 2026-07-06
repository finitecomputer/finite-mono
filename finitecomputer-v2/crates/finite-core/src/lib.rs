//! Shared types and business logic for the finite control plane.
//!
//! The target architecture is:
//! - `finite-core`: shared policy, schema, render, and runtime-control logic
//! - `finited`: host-side API/service
//! - `finitec`: CLI client and host-local break-glass entry point

pub mod chat;
pub mod chat_runtime;
pub mod cluster;
pub mod control_plane;
pub mod finite_chat;
pub mod models;
pub mod relay;
pub mod render;
pub mod util;

pub use chat::{
    CHAT_ATTACHMENT_BLOB_METADATA_KEY, ChatBootstrapData, ChatCapability, ChatInboxEvent,
    ChatInboxPage, ChatMachine, ChatMessage, ChatMessageAttachment, ChatMessageAttachmentInput,
    ChatMessagePage, ChatProjectAgent, ChatSite, ChatSkill, ChatSlashCommand, ChatThread, ChatUser,
    CreateChatThreadRequest, GatewayAckRequest, GatewayEditMessageRequest, GatewayMessageEvent,
    GatewayOutboundMessageRequest, GatewayPollRequest, GatewayPollResponse, SendChatMessageRequest,
};
pub use chat_runtime::{
    ChatAttachmentBlobUpload, ChatAttachmentData, ChatGetAttachmentRequest,
    ChatListMessagesRequest, ChatLogSyncCursor, ChatRuntime, ChatRuntimeHealth,
    ChatSendMessageRequest, resolve_state_dir,
};
pub use cluster::ClusterConfig;
pub use control_plane::{ControlPlane, ControlPlanePaths};
pub use finite_chat::{
    FiniteChatAppendInput, FiniteChatBridgeError, FiniteChatCommandRequestPayload,
    FiniteChatCommandResultPayload, FiniteChatCommandStatus, FiniteChatEventPayload,
    FiniteChatMessagePayload, FiniteChatRoomSeed, FiniteChatSegmentStartPayload, finitechat_device,
    finitechat_encode_app_event, finitechat_ensure_local_room_ready, finitechat_protocol_object_id,
    finitechat_push_app_event,
};
pub use models::{
    AuthenticateMachineTokenInput, AuthenticatedMachine, ClaimInviteInput, ConsumeOAuthStateInput,
    ControlPlaneDump, CoreExistingHostProjectImportRecord, CoreImportManifestOutput,
    CreateOAuthStateInput, EndpointAuth, EnsureGiteaCollaboratorInput, EnsureGiteaMachineUserInput,
    EnsureGiteaRepoInput, GiteaMachineAccessRecord, GiteaRepoRecord, InviteRecord,
    KnownExternalChannelParticipantRecord, ListGiteaReposInput, ListGiteaReposOutput,
    ListPublishedEndpointsInput, MachineHostnameInput, MachineIdInput, OAuthStateRecord,
    ProvisionMachineInput, PublishEndpointInput, PublishedEndpointRecord,
    PublishedEndpointRuntimeRecord, RenderManifestsOutput, ReservePublishedHostnameInput,
    RuntimeCodexPendingStatus, RuntimeCodexStartOutput, RuntimeCodexStatus,
    RuntimeGoogleWorkspaceStatus, RuntimeProfile, RuntimePublishedAppState,
    RuntimePublishedAppsStatusOutput, RuntimeUploadFileInput, RuntimeUploadFilesInput,
    RuntimeUploadFilesOutput, SimpleOk, SiteAuthUpdateInput, UnpublishEndpointInput,
    UpdateGiteaRepoAuthInput, UpdateRuntimeProfileInput, UploadedFileRecord, WorkloadRecord,
};
pub use relay::{
    CreateRelayChatConversationInput, CreateRelayEventInput, PreparedRelayChatAttachmentUpload,
    RelayAckOutput, RelayBridgeDevice, RelayChatAttachmentData, RelayChatBlobAck, RelayChatLogAck,
    RelayChatSnapshot, RelayChatStreamEvent, RelayCommandEnvelope, RelayCommandScope, RelayEvent,
    RelayEventsOutput, RelayHeartbeat, RelayResult, RelayResultAck, RelayStatusSnapshot,
    RelayStore, SendRelayChatMessageInput, StoreRelayChatLogInput, StoreRelayChatSnapshotInput,
    StoreRelayResultInput, StoreRelayStatusSnapshotInput, UpdateRelayChatConversationInput,
    prepare_relay_chat_attachment_upload,
};
