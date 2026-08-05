use super::*;
use crate::BrainKind;

pub const FOLDER_CONVENTION_DIRECTORIES: [&str; 5] =
    ["raw", "wiki", "inventory", "datasets", "output"];

pub fn folder_agent_instructions(folder_id: &str) -> String {
    format!(
        "# Folder Agent Instructions\n\nFolder id: `{folder_id}`\n\nUse `raw/` for Markdown source captures and Asset Source Notes, `wiki/` for durable synthesized pages, `inventory/` for source candidates and open questions, `datasets/` for manifests and query recipes, and `output/` for generated artifacts. Keep non-Markdown Asset bytes outside the Brain. Represent each Asset with one Markdown Source Note under `raw/` whose frontmatter includes `type`, `title`, and its canonical `resource` URI, then cite that note from synthesized work.\n"
    )
}

pub fn folder_convention_marker(folder_id: &str, convention: &str) -> String {
    format!("# {convention}\n\nAgent convention directory for Folder `{folder_id}`.\n")
}

/// Materialize already-opened content into a Brain Working Tree file map.
pub fn materialize_brain_working_tree(
    input: WorkingTreeMaterializeInput,
) -> Result<WorkingTreeProjection, PortabilityError> {
    let mut files = BTreeMap::new();
    let binary_files = BTreeMap::new();
    let mut folder_roots = BTreeMap::<(Option<BrainId>, FolderId), WorkingTreeFolderRoot>::new();
    let mut objects = Vec::new();
    let mut folder_paths = input
        .brain
        .folders
        .iter()
        .map(|folder| folder.path.to_string())
        .collect::<BTreeSet<_>>();
    folder_paths.extend(
        input
            .opened_pages
            .iter()
            .map(|page| page.folder_display_path.to_string()),
    );
    folder_paths.extend(
        input
            .locked_folders
            .iter()
            .map(|folder| folder.display_path.to_string()),
    );

    for page in &input.opened_pages {
        folder_roots
            .entry((page.source_brain_id.clone(), page.folder_id.clone()))
            .or_insert_with(|| WorkingTreeFolderRoot {
                folder_id: page.folder_id.to_string(),
                source_brain_id: page.source_brain_id.as_ref().map(ToString::to_string),
                path: page.folder_display_path.to_string(),
                can_read: true,
                metadata_only: false,
            });

        let full_path = working_tree_page_path(&page.folder_display_path, &page.page_path)?;
        if folder_paths.contains(&full_path) {
            return Err(PortabilityError::WorkingTreePathCollision { path: full_path });
        }
        insert_working_tree_file(&mut files, &full_path, page.markdown.clone())?;
        objects.push(WorkingTreeObjectManifestEntry {
            folder_id: page.folder_id.to_string(),
            source_brain_id: page.source_brain_id.as_ref().map(ToString::to_string),
            path: page.page_path.to_string(),
            object_id: page.object_id.as_str().to_owned(),
            revision: page.revision,
            key_version: page.key_version,
            content_type: page.content_type.clone(),
            content_hash: sha256_hex(page.markdown.as_bytes()),
        });
    }

    // `opened_assets` remains in the compatibility input while pre-hard-cut
    // clients may still identify legacy inline Asset plaintext. Current
    // projections deliberately omit those bytes and their manifest entries.

    for locked in &input.locked_folders {
        folder_roots
            .entry((locked.source_brain_id.clone(), locked.folder_id.clone()))
            .or_insert_with(|| WorkingTreeFolderRoot {
                folder_id: locked.folder_id.to_string(),
                source_brain_id: locked.source_brain_id.as_ref().map(ToString::to_string),
                path: locked.display_path.to_string(),
                can_read: false,
                metadata_only: true,
            });
        let locked_marker = serde_json::to_string_pretty(&serde_json::json!({
            "folderId": locked.folder_id.to_string(),
            "reason": safe_locked_reason(&locked.reason),
            "metadataOnly": true
        }))
        .expect("locked marker serializes");
        insert_working_tree_file(
            &mut files,
            &format!(
                ".finitebrain/encrypted-sync/folders/{}/locked.json",
                locked.folder_id
            ),
            locked_marker,
        )?;
    }

    let mut roots = folder_roots.into_values().collect::<Vec<_>>();
    objects.sort_by(|left, right| {
        left.source_brain_id
            .cmp(&right.source_brain_id)
            .then(left.folder_id.cmp(&right.folder_id))
            .then(left.path.cmp(&right.path))
    });
    roots.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.source_brain_id.cmp(&right.source_brain_id))
            .then(left.folder_id.cmp(&right.folder_id))
    });

    let directory = BrainDirectoryManifest {
        version: "finite-brain-directory-v1".to_owned(),
        brain: BrainDirectoryBrainSummary {
            id: input.brain.id.to_string(),
            kind: format!("{:?}", input.brain.kind).to_lowercase(),
            name: input.brain.name.to_string(),
            owner_npub: input.brain.owner_user_id.as_ref().map(ToString::to_string),
        },
        working_tree: BrainDirectoryPath {
            path: ".".to_owned(),
        },
        encrypted_sync: BrainDirectoryPath {
            path: ".finitebrain/encrypted-sync".to_owned(),
        },
        portability: BrainDirectoryPortability {
            owned_by_agent_runtime: false,
            owned_by_app_surface: false,
        },
        created_at: input.generated_at.clone(),
        updated_at: input.generated_at.clone(),
    };
    let state = BrainWorkingTreeStateManifest {
        version: "finite-brain-working-tree-state-v1".to_owned(),
        folder_roots: roots,
        objects,
        sync: WorkingTreeSyncState {
            latest_sequence: input.latest_sequence,
        },
    };

    insert_working_tree_file(
        &mut files,
        ".finitebrain/brain-directory.json",
        serde_json::to_string_pretty(&directory).expect("directory manifest serializes"),
    )?;
    insert_working_tree_file(
        &mut files,
        ".finitebrain/working-tree-state.json",
        serde_json::to_string_pretty(&state).expect("working-tree manifest serializes"),
    )?;
    insert_working_tree_file(
        &mut files,
        "AGENTS.md",
        root_agents_file(&input.brain, &input.generated_by_npub, &input.acting_role),
    )?;
    insert_working_tree_file(&mut files, "_index.md", root_working_tree_index(&state))?;
    insert_working_tree_file(
        &mut files,
        "_wiki/index.md",
        root_wiki_index(&input.generated_at, &input.generated_by_npub, &state),
    )?;
    insert_working_tree_file(
        &mut files,
        "_wiki/backlinks.md",
        "# Backlinks\n\n".to_owned(),
    )?;
    insert_working_tree_file(&mut files, "_wiki/orphans.md", root_orphans_report(&state))?;
    insert_working_tree_file(
        &mut files,
        "_wiki/stale.md",
        root_stale_report(&input.generated_at),
    )?;
    let tags_report = root_tags_report(&files);
    insert_working_tree_file(&mut files, "_wiki/tags.md", tags_report)?;

    for root in &state.folder_roots {
        if !root.can_read {
            continue;
        }
        insert_working_tree_file_if_absent(&mut files, &format!("{}/AGENTS.md", root.path), || {
            folder_agent_instructions(&root.folder_id)
        });
        insert_working_tree_file_if_absent(&mut files, &format!("{}/_index.md", root.path), || {
            folder_index_file(root, &state)
        });
        insert_working_tree_file_if_absent(
            &mut files,
            &format!("{}/_wiki/index.md", root.path),
            || folder_wiki_index(root, &input.generated_at, &input.generated_by_npub, &state),
        );
        for convention in FOLDER_CONVENTION_DIRECTORIES {
            insert_working_tree_file_if_absent(
                &mut files,
                &format!("{}/{convention}/.keep", root.path),
                || folder_convention_marker(&root.folder_id, convention),
            );
        }
    }

    Ok(WorkingTreeProjection {
        files,
        binary_files,
        directory,
        state,
    })
}

/// Translate local working-tree changes into Product Client encrypted-sync intents.
pub fn plan_working_tree_change_intents(
    state: &BrainWorkingTreeStateManifest,
    changes: &[WorkingTreeChange],
) -> Vec<WorkingTreeChangeIntent> {
    changes
        .iter()
        .map(|change| match change {
            WorkingTreeChange::Upsert { path, markdown } => {
                plan_working_tree_upsert(state, path, markdown)
            }
            WorkingTreeChange::UpsertAsset {
                path,
                bytes,
                content_type,
                has_source_note,
            } => plan_working_tree_asset_upsert(state, path, bytes, content_type, *has_source_note),
            WorkingTreeChange::Rename { from_path, to_path } => {
                plan_working_tree_rename(state, from_path, to_path)
            }
            WorkingTreeChange::Delete { path } => plan_working_tree_delete(state, path),
        })
        .collect()
}

fn working_tree_page_path(
    folder_path: &SafeRelativePath,
    page_path: &SafeRelativePath,
) -> Result<String, CoreError> {
    let path = format!("{}/{}", folder_path.as_str(), page_path.as_str());
    SafeRelativePath::new("working_tree_path", &path)?;
    Ok(path)
}

fn insert_working_tree_file(
    files: &mut BTreeMap<String, String>,
    path: &str,
    content: String,
) -> Result<(), PortabilityError> {
    validate_working_tree_file_path(path)?;
    let path = path.to_owned();
    if files.insert(path.clone(), content).is_some() {
        return Err(PortabilityError::WorkingTreePathCollision { path });
    }
    Ok(())
}

fn insert_working_tree_file_if_absent(
    files: &mut BTreeMap<String, String>,
    path: &str,
    content: impl FnOnce() -> String,
) {
    files.entry(path.to_owned()).or_insert_with(content);
}

fn validate_working_tree_file_path(path: &str) -> Result<(), CoreError> {
    if path.starts_with(".finitebrain/") {
        if path.contains('\\')
            || path.chars().any(|c| c == '\0' || c.is_control())
            || path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(CoreError::InvalidPath {
                field: "working_tree_file",
                value: path.to_owned(),
            });
        }
        return Ok(());
    }
    SafeRelativePath::new("working_tree_file", path).map(|_| ())
}

fn safe_locked_reason(reason: &str) -> &'static str {
    match reason {
        "missing-folder-key" => "missing-folder-key",
        "no-folder-access" => "no-folder-access",
        _ => "inaccessible",
    }
}

fn root_agents_file(brain: &Brain, actor: &UserId, acting_role: &str) -> String {
    let kind = match brain.kind {
        BrainKind::Personal => "Personal Brain",
        BrainKind::Organization => "Organization Brain",
    };
    format!(
        "# FiniteBrain {kind} Working Tree\n\nBrain ID: `{brain_id}`\nActing Member Identity: `{actor}`\nActing Brain role: `{acting_role}`\n\n- Read and write the materialized Folders available to this identity and role.\n- Keep non-Markdown Asset bytes outside the Brain and represent each one with a Markdown Asset Source Note under the Folder's `raw/` tree.\n- Do not write decrypted content into `.finitebrain/encrypted-sync`.\n- Changes must be returned through the Product Client encrypted sync path.\n",
        brain_id = brain.id,
    )
}

fn root_working_tree_index(state: &BrainWorkingTreeStateManifest) -> String {
    let folders = state
        .folder_roots
        .iter()
        .map(|root| {
            if root.can_read {
                format!("- [{}]({}/_index.md)", root.path, root.path)
            } else {
                format!("- {} (locked metadata only)", root.path)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("# Brain Working Tree\n\n## Folders\n\n{folders}\n")
}

fn root_wiki_index(
    generated_at: &str,
    generated_by: &UserId,
    state: &BrainWorkingTreeStateManifest,
) -> String {
    let readable_count = state
        .folder_roots
        .iter()
        .filter(|root| root.can_read)
        .count();
    format!(
        "# Working Tree Wiki\n\nGenerated at: {generated_at}\nGenerated by: {generated_by}\nReadable Folders: {readable_count}\nReadable Objects: {}\n",
        state.objects.len()
    )
}

fn root_orphans_report(state: &BrainWorkingTreeStateManifest) -> String {
    let pages = state
        .objects
        .iter()
        .map(|object| format!("- {}/{}", object.folder_id, object.path))
        .collect::<Vec<_>>()
        .join("\n");
    format!("# Orphans\n\n{pages}\n")
}

fn root_stale_report(generated_at: &str) -> String {
    format!("# Stale\n\nGenerated at: {generated_at}\nNo stale-page policy was applied.\n")
}

fn root_tags_report(files: &BTreeMap<String, String>) -> String {
    format!("# Tags\n\n{}\n", collect_tags(files).join("\n"))
}

fn folder_index_file(
    root: &WorkingTreeFolderRoot,
    state: &BrainWorkingTreeStateManifest,
) -> String {
    let pages = state
        .objects
        .iter()
        .filter(|object| object_belongs_to_root(object, root))
        .map(|object| format!("- [{}]({})", object.path, object.path))
        .collect::<Vec<_>>()
        .join("\n");
    format!("# {}\n\n{pages}\n", root.path)
}

fn folder_wiki_index(
    root: &WorkingTreeFolderRoot,
    generated_at: &str,
    generated_by: &UserId,
    state: &BrainWorkingTreeStateManifest,
) -> String {
    let count = state
        .objects
        .iter()
        .filter(|object| object_belongs_to_root(object, root))
        .count();
    format!(
        "# Folder Wiki\n\nGenerated at: {generated_at}\nGenerated by: {generated_by}\nFolder: {}\nReadable Objects: {count}\n",
        root.path
    )
}

fn plan_working_tree_upsert(
    state: &BrainWorkingTreeStateManifest,
    path: &SafeRelativePath,
    markdown: &str,
) -> WorkingTreeChangeIntent {
    let Some((root, local_path)) = resolve_working_tree_folder(state, path) else {
        return unresolved_intent("path is outside materialized readable Folders");
    };
    if !root.can_read {
        return unresolved_intent("Folder is locked in this working tree");
    }
    let source_brain_id = match source_brain_id_for_root(root) {
        Ok(source_brain_id) => source_brain_id,
        Err(reason) => return unresolved_intent(&reason),
    };
    let existing = object_for_local_path(state, root, &local_path);
    let object_id = existing
        .and_then(|object| ObjectId::new(object.object_id.clone()).ok())
        .unwrap_or_else(|| generated_working_tree_object_id(&root.folder_id, &local_path));
    WorkingTreeChangeIntent {
        action: if existing.is_some() {
            WorkingTreeIntentAction::Update
        } else {
            WorkingTreeIntentAction::Create
        },
        route: WorkingTreeIntentRoute::EncryptedObjectWrite,
        folder_id: FolderId::new(root.folder_id.clone()).ok(),
        source_brain_id,
        object_id: Some(object_id),
        target_path: Some(local_path),
        from_path: None,
        base_revision: existing.map(|object| object.revision),
        content: Some(WorkingTreeIntentContent::PageMarkdown(markdown.to_owned())),
        reason: None,
    }
}

fn plan_working_tree_asset_upsert(
    _state: &BrainWorkingTreeStateManifest,
    _path: &SafeRelativePath,
    _bytes: &[u8],
    _content_type: &str,
    _has_source_note: bool,
) -> WorkingTreeChangeIntent {
    unresolved_intent(
        "non-Markdown files are not stored in the Brain; create a Markdown Asset Source Note under raw/ with a canonical resource pointer",
    )
}

fn plan_working_tree_rename(
    state: &BrainWorkingTreeStateManifest,
    from_path: &SafeRelativePath,
    to_path: &SafeRelativePath,
) -> WorkingTreeChangeIntent {
    let Some((from_root, from_local_path)) = resolve_working_tree_folder(state, from_path) else {
        return unresolved_intent("source path is outside materialized readable Folders");
    };
    let Some((to_root, to_local_path)) = resolve_working_tree_folder(state, to_path) else {
        return unresolved_intent("destination path is outside materialized readable Folders");
    };
    if !from_root.can_read || !to_root.can_read {
        return unresolved_intent("Folder is locked in this working tree");
    }
    if from_root.folder_id != to_root.folder_id {
        return unresolved_intent("cross-Folder moves require Brain Admin handling");
    }
    if from_root.source_brain_id != to_root.source_brain_id {
        return unresolved_intent("cross-Brain moves require Brain Admin handling");
    }
    let source_brain_id = match source_brain_id_for_root(from_root) {
        Ok(source_brain_id) => source_brain_id,
        Err(reason) => return unresolved_intent(&reason),
    };
    let Some(existing) = object_for_local_path(state, from_root, &from_local_path) else {
        return unresolved_intent("source object is not in working-tree state");
    };
    WorkingTreeChangeIntent {
        action: WorkingTreeIntentAction::Move,
        route: WorkingTreeIntentRoute::EncryptedObjectMove,
        folder_id: FolderId::new(from_root.folder_id.clone()).ok(),
        source_brain_id,
        object_id: ObjectId::new(existing.object_id.clone()).ok(),
        target_path: Some(to_local_path),
        from_path: Some(from_local_path),
        base_revision: Some(existing.revision),
        content: None,
        reason: None,
    }
}

fn plan_working_tree_delete(
    state: &BrainWorkingTreeStateManifest,
    path: &SafeRelativePath,
) -> WorkingTreeChangeIntent {
    let Some((root, local_path)) = resolve_working_tree_folder(state, path) else {
        return unresolved_intent("path is outside materialized readable Folders");
    };
    if !root.can_read {
        return unresolved_intent("Folder is locked in this working tree");
    }
    let source_brain_id = match source_brain_id_for_root(root) {
        Ok(source_brain_id) => source_brain_id,
        Err(reason) => return unresolved_intent(&reason),
    };
    let Some(existing) = object_for_local_path(state, root, &local_path) else {
        return unresolved_intent("object is not in working-tree state");
    };
    WorkingTreeChangeIntent {
        action: WorkingTreeIntentAction::Delete,
        route: WorkingTreeIntentRoute::EncryptedObjectDelete,
        folder_id: FolderId::new(root.folder_id.clone()).ok(),
        source_brain_id,
        object_id: ObjectId::new(existing.object_id.clone()).ok(),
        target_path: Some(local_path),
        from_path: None,
        base_revision: Some(existing.revision),
        content: None,
        reason: None,
    }
}

fn resolve_working_tree_folder<'a>(
    state: &'a BrainWorkingTreeStateManifest,
    path: &SafeRelativePath,
) -> Option<(&'a WorkingTreeFolderRoot, SafeRelativePath)> {
    state
        .folder_roots
        .iter()
        .filter_map(|root| {
            let root_path = root.path.as_str();
            let relative = path
                .as_str()
                .strip_prefix(root_path)
                .and_then(|rest| rest.strip_prefix('/'))?;
            SafeRelativePath::new("working_tree_local_path", relative)
                .ok()
                .map(|local_path| (root, local_path))
        })
        .max_by_key(|(root, _)| root.path.len())
}

fn object_for_local_path<'a>(
    state: &'a BrainWorkingTreeStateManifest,
    root: &WorkingTreeFolderRoot,
    local_path: &SafeRelativePath,
) -> Option<&'a WorkingTreeObjectManifestEntry> {
    state.objects.iter().find(|object| {
        object_belongs_to_root(object, root) && object.path == local_path.to_string()
    })
}

fn object_belongs_to_root(
    object: &WorkingTreeObjectManifestEntry,
    root: &WorkingTreeFolderRoot,
) -> bool {
    object.folder_id == root.folder_id && object.source_brain_id == root.source_brain_id
}

fn source_brain_id_for_root(root: &WorkingTreeFolderRoot) -> Result<Option<BrainId>, String> {
    root.source_brain_id
        .as_ref()
        .map(|source_brain_id| {
            BrainId::new(source_brain_id.clone()).map_err(|error| error.to_string())
        })
        .transpose()
}

fn generated_working_tree_object_id(folder_id: &str, local_path: &SafeRelativePath) -> ObjectId {
    let digest = Sha256::digest(format!("{folder_id}/{}", local_path.as_str()).as_bytes());
    let hex = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ObjectId::new(format!("obj_{hex}")).expect("generated object id is valid")
}

fn unresolved_intent(reason: &str) -> WorkingTreeChangeIntent {
    WorkingTreeChangeIntent {
        action: WorkingTreeIntentAction::Unresolved,
        route: WorkingTreeIntentRoute::Unresolved,
        folder_id: None,
        source_brain_id: None,
        object_id: None,
        target_path: None,
        from_path: None,
        base_revision: None,
        content: None,
        reason: Some(reason.to_owned()),
    }
}
