use super::*;

struct LinkEdge {
    from: String,
    to: String,
}

/// Export accessible decrypted pages into a readable OKF bundle.
pub fn export_okf_bundle(input: OkfExportInput) -> Result<OkfBundle, PortabilityError> {
    let mut files = BTreeMap::new();
    let mut manifest_objects = Vec::new();
    let mut manifest_folders = Vec::new();
    let mut page_bundle_paths = BTreeMap::new();
    let mut opened_page_paths = BTreeSet::new();
    let mut bundle_paths = BTreeSet::new();

    for page in &input.opened_pages {
        let page_key = (page.folder_id.clone(), page.page_path.as_str().to_owned());
        if !opened_page_paths.insert(page_key.clone()) {
            return Err(PortabilityError::DuplicatePagePath {
                folder_id: page.folder_id.to_string(),
                path: page.page_path.to_string(),
            });
        }
        let bundle_path = content_bundle_path(page)?;
        if !bundle_paths.insert(bundle_path.clone()) {
            return Err(PortabilityError::DuplicateBundlePath { path: bundle_path });
        }
        page_bundle_paths.insert(page_key, bundle_path);
    }

    let mut link_edges = Vec::new();
    for page in &input.opened_pages {
        let bundle_path = page_bundle_paths
            .get(&(page.folder_id.clone(), page.page_path.as_str().to_owned()))
            .expect("page path indexed")
            .clone();
        let (rewritten, links) = rewrite_markdown_links(page, &bundle_path, &page_bundle_paths);
        link_edges.extend(links);
        manifest_objects.push(OkfObjectManifestEntry {
            folder_id: page.folder_id.to_string(),
            object_id: page.object_id.as_str().to_owned(),
            path: bundle_path.clone(),
            content_type: page.content_type.clone(),
            content_hash: sha256_hex(rewritten.as_bytes()),
        });
        files.insert(bundle_path, rewritten);
    }

    let source_folders = input
        .source_brain
        .folders
        .iter()
        .map(|folder| (folder.id.clone(), folder))
        .collect::<BTreeMap<_, _>>();
    let accessible_folder_paths = input
        .opened_pages
        .iter()
        .map(|page| (page.folder_id.clone(), page.folder_display_path.to_string()))
        .collect::<BTreeMap<_, _>>();
    for (folder_id, display_path) in accessible_folder_paths {
        if let Some(folder) = source_folders.get(&folder_id) {
            manifest_folders.push(OkfFolderManifestEntry {
                folder_id: folder_id.to_string(),
                display_path,
                access: folder.access,
                omitted: false,
            });
        }
    }
    for omission in &input.omissions {
        if let Some(folder) = source_folders.get(&omission.folder_id) {
            manifest_folders.push(OkfFolderManifestEntry {
                folder_id: omission.folder_id.to_string(),
                display_path: omission.display_path.to_string(),
                access: folder.access,
                omitted: true,
            });
        }
    }

    let omissions = input
        .omissions
        .into_iter()
        .map(|omission| OkfOmissionManifestEntry {
            folder_id: omission.folder_id.to_string(),
            display_path: omission.display_path.to_string(),
            reason: safe_locked_reason(&omission.reason).to_owned(),
        })
        .collect::<Vec<_>>();

    let wiki_files = generated_wiki_files(
        &input.exported_at,
        &input.exported_by_npub,
        &files,
        &link_edges,
    )?;
    for (path, body) in wiki_files {
        if files.insert(path.clone(), body).is_some() {
            return Err(PortabilityError::DuplicateBundlePath { path });
        }
    }

    let manifest = OkfManifest {
        version: "finite-okf-brain-export-v1".to_owned(),
        exported_at: input.exported_at,
        exported_by_npub: input.exported_by_npub.to_string(),
        source_brain: OkfSourceBrain {
            id: input.source_brain.id.to_string(),
            kind: format!("{:?}", input.source_brain.kind).to_lowercase(),
            name: input.source_brain.name.to_string(),
        },
        folders: manifest_folders,
        objects: manifest_objects,
        omissions,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
    files.insert("okf-brain.json".to_owned(), manifest_json);

    Ok(OkfBundle { manifest, files })
}

/// Plan readable OKF import conflict handling before client-side encryption/upload.
pub fn plan_okf_import(
    pages: &[OkfImportPage],
    existing_pages: &[ExistingPagePath],
    mode: OkfConflictMode,
) -> Result<OkfImportPlan, PortabilityError> {
    let mut occupied = existing_pages
        .iter()
        .map(|page| (page.folder_id.clone(), page.page_path.to_string()))
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();

    for page in pages {
        let key = (page.folder_id.clone(), page.target_path.to_string());
        let collides = occupied.contains(&key);
        match (mode, collides) {
            (_, false) => {
                occupied.insert(key);
                entries.push(OkfImportPlanEntry {
                    source_path: page.source_path.clone(),
                    folder_id: page.folder_id.clone(),
                    target_path: page.target_path.clone(),
                    action: OkfImportAction::Create,
                });
            }
            (OkfConflictMode::Skip, true) => entries.push(OkfImportPlanEntry {
                source_path: page.source_path.clone(),
                folder_id: page.folder_id.clone(),
                target_path: page.target_path.clone(),
                action: OkfImportAction::Skip,
            }),
            (OkfConflictMode::Copy, true) => {
                let copy_path = unique_copy_path(&page.folder_id, &page.target_path, &occupied)?;
                occupied.insert((page.folder_id.clone(), copy_path.to_string()));
                entries.push(OkfImportPlanEntry {
                    source_path: page.source_path.clone(),
                    folder_id: page.folder_id.clone(),
                    target_path: copy_path,
                    action: OkfImportAction::Copy,
                });
            }
            (OkfConflictMode::Overwrite { confirmed: false }, true) => {
                return Err(PortabilityError::OverwriteRequiresConfirmation);
            }
            (OkfConflictMode::Overwrite { confirmed: true }, true) => {
                entries.push(OkfImportPlanEntry {
                    source_path: page.source_path.clone(),
                    folder_id: page.folder_id.clone(),
                    target_path: page.target_path.clone(),
                    action: OkfImportAction::Overwrite,
                });
            }
        }
    }

    Ok(OkfImportPlan { entries })
}

fn content_bundle_path(page: &OpenedPage) -> Result<String, CoreError> {
    let path = format!(
        "content/{}/{}",
        page.folder_display_path.as_str(),
        page.page_path.as_str()
    );
    SafeRelativePath::new("okf_path", &path)?;
    Ok(path)
}

fn rewrite_markdown_links(
    page: &OpenedPage,
    current_bundle_path: &str,
    page_bundle_paths: &BTreeMap<(FolderId, String), String>,
) -> (String, Vec<LinkEdge>) {
    let mut output = String::new();
    let mut links = Vec::new();
    let mut rest = page.markdown.as_str();

    while let Some(open) = rest.find('[') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find("](") else {
            output.push_str(&rest[open..]);
            return (output, links);
        };
        let label = &after_open[..close];
        let after_marker = &after_open[close + 2..];
        let Some(end) = after_marker.find(')') else {
            output.push_str(&rest[open..]);
            return (output, links);
        };
        let target = &after_marker[..end];
        let original = &rest[open..open + 1 + close + 2 + end + 1];
        if is_external_or_anchor(target) {
            output.push_str(original);
        } else if let Some(resolved) = resolve_relative_path(page.page_path.as_str(), target) {
            let key = (page.folder_id.clone(), resolved);
            if let Some(target_bundle_path) = page_bundle_paths.get(&key) {
                let relative = relative_path_between(current_bundle_path, target_bundle_path);
                output.push_str(&format!("[{label}]({relative})"));
                links.push(LinkEdge {
                    from: current_bundle_path.to_owned(),
                    to: target_bundle_path.clone(),
                });
            } else {
                output.push_str(label);
            }
        } else {
            output.push_str(label);
        }
        rest = &after_marker[end + 1..];
    }

    output.push_str(rest);
    (output, links)
}

fn is_external_or_anchor(target: &str) -> bool {
    target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
}

fn resolve_relative_path(base_page_path: &str, target: &str) -> Option<String> {
    if target.starts_with('/') || target.contains('\\') {
        return None;
    }
    let target = target.split('#').next().unwrap_or(target);
    let mut segments = base_page_path.split('/').collect::<Vec<_>>();
    segments.pop();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            value => segments.push(value),
        }
    }
    let path = segments.join("/");
    SafeRelativePath::new("markdown_link", &path).ok()?;
    Some(path)
}

fn relative_path_between(from_file: &str, to_file: &str) -> String {
    let mut from = from_file.split('/').collect::<Vec<_>>();
    from.pop();
    let to = to_file.split('/').collect::<Vec<_>>();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = Vec::new();
    parts.extend(std::iter::repeat_n("..", from.len().saturating_sub(common)));
    parts.extend(to[common..].iter().copied());
    parts.join("/")
}

fn generated_wiki_files(
    exported_at: &str,
    exported_by: &UserId,
    files: &BTreeMap<String, String>,
    links: &[LinkEdge],
) -> Result<BTreeMap<String, String>, CoreError> {
    let content_paths = files
        .keys()
        .filter(|path| path.starts_with("content/"))
        .cloned()
        .collect::<Vec<_>>();
    let incoming = links
        .iter()
        .map(|link| link.to.clone())
        .collect::<BTreeSet<_>>();

    let mut wiki = BTreeMap::new();
    wiki.insert(
        "_wiki/index.md".to_owned(),
        format!(
            "# OKF Index\n\nGenerated at: {exported_at}\nGenerated by: {exported_by}\n\n{}",
            content_paths
                .iter()
                .map(|path| format!(
                    "- [{}]({})",
                    path,
                    relative_path_between("_wiki/index.md", path)
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    wiki.insert(
        "_wiki/backlinks.md".to_owned(),
        format!(
            "# Backlinks\n\n{}",
            links
                .iter()
                .map(|link| format!("- {} -> {}", link.from, link.to))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    wiki.insert(
        "_wiki/orphans.md".to_owned(),
        format!(
            "# Orphans\n\n{}",
            content_paths
                .iter()
                .filter(|path| !incoming.contains(*path))
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    wiki.insert(
        "_wiki/tags.md".to_owned(),
        format!("# Tags\n\n{}", collect_tags(files).join("\n")),
    );
    wiki.insert(
        "_wiki/stale.md".to_owned(),
        format!("# Stale\n\nGenerated at: {exported_at}\nNo stale-page policy was applied."),
    );

    for path in wiki.keys() {
        SafeRelativePath::new("wiki_path", path)?;
    }
    Ok(wiki)
}

fn unique_copy_path(
    folder_id: &FolderId,
    path: &SafeRelativePath,
    occupied: &BTreeSet<(FolderId, String)>,
) -> Result<SafeRelativePath, CoreError> {
    let value = path.as_str();
    let (stem, extension) = value
        .strip_suffix(".md")
        .map_or((value, ""), |stem| (stem, ".md"));
    for index in 1..=1_000 {
        let suffix = if index == 1 {
            " imported".to_owned()
        } else {
            format!(" imported {index}")
        };
        let candidate = format!("{stem}{suffix}{extension}");
        if !occupied.contains(&(folder_id.clone(), candidate.clone())) {
            return SafeRelativePath::new("copy_path", candidate);
        }
    }
    SafeRelativePath::new("copy_path", format!("{stem} imported overflow{extension}"))
}
