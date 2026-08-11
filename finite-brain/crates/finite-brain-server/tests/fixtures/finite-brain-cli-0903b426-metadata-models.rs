// Frozen verbatim excerpt from finite-brain-cli/src/models.rs at
// 0903b4267efbd53c8eafad65ea42958f860ebc0c (the rollout merge base).
// Do not edit this fixture in-place; add a newly versioned fixture instead.

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisibleBrainsResponse {
    pub(crate) brains: Vec<VisibleBrainSummary>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisibleBrainSummary {
    pub(crate) brain_id: String,
    pub(crate) kind: String,
    pub(crate) role: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrainMetadataView {
    pub(crate) brain_id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) owner_user_id: Option<String>,
    #[serde(default)]
    pub(crate) personal_agent: Option<PersonalAgentView>,
    pub(crate) members: Vec<String>,
    #[serde(default)]
    pub(crate) guests: Vec<String>,
    pub(crate) admins: Vec<String>,
    pub(crate) folders: Vec<FolderMetadataView>,
    #[serde(default)]
    pub(crate) mounted_folders: Vec<MountedFolderMetadataView>,
    #[serde(default)]
    pub(crate) grant_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersonalAgentView {
    pub(crate) agent_npub: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderMetadataView {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default = "default_folder_role")]
    pub(crate) role: String,
    pub(crate) access: String,
    #[serde(default)]
    pub(crate) parent_folder_id: Option<String>,
    pub(crate) path: String,
    pub(crate) access_user_ids: Vec<String>,
    pub(crate) current_key_version: u32,
    #[serde(default)]
    pub(crate) setup_incomplete: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MountedFolderMetadataView {
    pub(crate) mount_id: String,
    pub(crate) source_brain_id: String,
    pub(crate) source_folder_id: String,
    pub(crate) display_name: String,
    pub(crate) display_parent_folder_id: Option<String>,
    pub(crate) state: String,
}

fn default_folder_role() -> String {
    "folder".to_owned()
}
