use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeProfile {
    pub id: String,
    pub label: String,
    pub image_name: String,
    pub image_tag: String,
    pub feature_set: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointAuth {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadOpencode {
    pub port: u16,
    pub hostname: String,
    pub project_dir: String,
    pub auth: EndpointAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadSsh {
    pub enable: bool,
    pub node_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadRecord {
    pub id: String,
    pub owner: String,
    pub owner_email: Option<String>,
    pub namespace: String,
    pub runtime_profile: String,
    pub home_volume_size: String,
    pub opencode: WorkloadOpencode,
    pub ssh: WorkloadSsh,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InviteRecord {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub email: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "claimToken")]
    pub claim_token: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "claimedAt", skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishedEndpointRecord {
    #[serde(
        rename = "machineId",
        skip_serializing_if = "String::is_empty",
        default
    )]
    pub machine_id: String,
    pub hostname: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_port: Option<u16>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_cwd: Option<String>,
    pub desired_process_state: String,
    pub auth: EndpointAuth,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeImageRevisionDump {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GiteaMachineAccessRecord {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub username: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub email: String,
    #[serde(rename = "hostUrl")]
    pub host_url: String,
    #[serde(rename = "cloneBaseUrl")]
    pub clone_base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GiteaRepoRecord {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub owner: String,
    pub name: String,
    pub private: bool,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
    #[serde(rename = "cloneUrl")]
    pub clone_url: String,
    #[serde(rename = "defaultBranch", skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    pub auth: EndpointAuth,
    #[serde(
        rename = "pendingEmails",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub pending_emails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneDump {
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub invites: Vec<InviteRecord>,
    #[serde(default)]
    pub workloads: Vec<WorkloadRecord>,
    #[serde(default, rename = "publishedEndpoints")]
    pub published_endpoints: Vec<PublishedEndpointRecord>,
    #[serde(default, rename = "runtimeImageRevisions")]
    pub runtime_image_revisions: BTreeMap<String, RuntimeImageRevisionDump>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreImportManifestOutput {
    pub source_host_id: String,
    pub records: Vec<CoreExistingHostProjectImportRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreExistingHostProjectImportRecord {
    pub source_host_id: String,
    pub source_machine_id: String,
    pub owner_email: Option<String>,
    pub display_name: String,
    pub hostname: Option<String>,
    pub runtime_host: Option<String>,
    pub runtime_status: String,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
    pub known_external_channel_participants: Vec<KnownExternalChannelParticipantRecord>,
    pub admin_visible_to_emails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownExternalChannelParticipantRecord {
    pub channel: String,
    pub external_user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProvisionMachineInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub email: String,
    #[serde(rename = "runtimeProfile")]
    pub runtime_profile: Option<String>,
    #[serde(rename = "homeVolumeSize")]
    pub home_volume_size: String,
    pub hostname: String,
    pub port: u16,
    #[serde(rename = "sshNodePort")]
    pub ssh_node_port: u16,
    #[serde(rename = "claimToken")]
    pub claim_token: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteAuthUpdateInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub mode: String,
    #[serde(rename = "ownerEmail")]
    pub owner_email: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(rename = "orgDomain")]
    pub org_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateRuntimeProfileInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "runtimeProfile")]
    pub runtime_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthStateRecord {
    pub state: String,
    pub provider: String,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "viewerEmail", skip_serializing_if = "Option::is_none")]
    pub viewer_email: Option<String>,
    #[serde(rename = "redirectPath")]
    pub redirect_path: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateOAuthStateInput {
    pub state: String,
    pub provider: String,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "viewerEmail")]
    pub viewer_email: Option<String>,
    #[serde(rename = "redirectPath")]
    pub redirect_path: String,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsumeOAuthStateInput {
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticateMachineTokenInput {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticatedMachine {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "ownerEmail", skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListPublishedEndpointsInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListPublishedEndpointsOutput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub endpoints: Vec<PublishedEndpointRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReservePublishedHostnameInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnsureGiteaMachineUserInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnsureGiteaRepoInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub private: bool,
    #[serde(rename = "autoInit", default)]
    pub auto_init: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListGiteaReposInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListGiteaReposOutput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub username: String,
    pub repos: Vec<GiteaRepoRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnsureGiteaCollaboratorInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "repoName")]
    pub repo_name: String,
    pub collaborator: String,
    #[serde(default = "default_gitea_permission")]
    pub permission: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateGiteaRepoAuthInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "repoName")]
    pub repo_name: String,
    pub mode: String,
    #[serde(rename = "ownerEmail")]
    pub owner_email: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(rename = "orgDomain")]
    pub org_domain: Option<String>,
    #[serde(rename = "confirmPublic")]
    pub confirm_public: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishEndpointInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub hostname: String,
    #[serde(rename = "targetPort")]
    pub target_port: u16,
    pub label: Option<String>,
    pub mode: Option<String>,
    #[serde(rename = "ownerEmail")]
    pub owner_email: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(rename = "orgDomain")]
    pub org_domain: Option<String>,
    #[serde(rename = "confirmPublic")]
    pub confirm_public: Option<String>,
    #[serde(rename = "runCommand")]
    pub run_command: Option<String>,
    #[serde(rename = "runCwd")]
    pub run_cwd: Option<String>,
    #[serde(rename = "desiredProcessState")]
    pub desired_process_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnpublishEndpointInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineHostnameInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub hostname: String,
}

fn default_true() -> bool {
    true
}

fn default_gitea_permission() -> String {
    "write".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineIdInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimInviteInput {
    #[serde(rename = "claimToken")]
    pub claim_token: String,
    #[serde(rename = "claimedAt")]
    pub claimed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimpleOk {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitOutput {
    pub ok: bool,
    pub bootstrapped: bool,
    #[serde(rename = "machineCount")]
    pub machine_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderManifestsOutput {
    pub ok: bool,
    pub rendered: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePublishedAppState {
    pub hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub managed: bool,
    pub desired_process_state: String,
    pub observed_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishedEndpointRuntimeRecord {
    #[serde(flatten)]
    pub endpoint: PublishedEndpointRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimePublishedAppState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePublishedAppsStatusOutput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub apps: Vec<PublishedEndpointRuntimeRecord>,
    #[serde(rename = "runtimeError", skip_serializing_if = "Option::is_none")]
    pub runtime_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeGoogleWorkspaceStatus {
    pub gws_installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gws_status: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gws_error: Option<String>,
    pub hermes_authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hermes_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hermes_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCodexPendingStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCodexStatus {
    pub codex_installed: bool,
    pub codex_runnable: bool,
    pub logged_in: bool,
    pub auth_file_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hermes_codex_usable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hermes_codex_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hermes_codex_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<RuntimeCodexPendingStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCodexStartOutput {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already_logged_in: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<RuntimeCodexPendingStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeUploadFileInput {
    #[serde(rename = "tempPath")]
    pub temp_path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeUploadFilesInput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub files: Vec<RuntimeUploadFileInput>,
    #[serde(rename = "destinationRelpath")]
    pub destination_relpath: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadedFileRecord {
    pub name: String,
    pub size: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeUploadFilesOutput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "destinationPath")]
    pub destination_path: String,
    pub files: Vec<UploadedFileRecord>,
}
