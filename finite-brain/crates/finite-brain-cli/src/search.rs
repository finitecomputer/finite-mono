use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use finite_brain_core::portability::{
    BrainWorkingTreeStateManifest, WorkingTreeFolderRoot, WorkingTreeObjectManifestEntry,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    AgentState, CliEnvironment, CliError, ConflictState, SyncChangeReport, SyncOnceReport,
    create_private_directory_if_missing, current_tree_root, read_agent_state,
    read_working_tree_state, set_private_file_permissions, write_json,
};

const SEARCH_INDEX_VERSION: &str = "finitebrain-folder-search-v3";
const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;
const MAX_INDEXED_MARKDOWN_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INDEXED_FILES_PER_FOLDER: usize = 100_000;
const MAX_SECTION_CHARS: usize = 12_000;
const SECTION_OVERLAP_CHARS: usize = 500;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchReport {
    query: String,
    mode: &'static str,
    searched_folders: Vec<String>,
    results: Vec<SearchEvidence>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchEvidence {
    rank: usize,
    folder_id: String,
    source_brain_id: Option<String>,
    folder_path: String,
    page_path: String,
    page_title: String,
    heading_ancestry: Vec<String>,
    heading: Option<String>,
    excerpt: String,
    disposition: SearchDisposition,
    signals: [&'static str; 1],
    #[serde(skip)]
    normalized_bm25: f64,
    #[serde(skip)]
    raw_bm25: f64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchDisposition {
    Synced,
    LocalOnly,
    Conflicted,
}

#[derive(Debug)]
struct SearchOptions {
    query: String,
    folders: Vec<String>,
    limit: usize,
}

#[derive(Debug)]
struct IndexedPage {
    path: String,
    content_hash: String,
    disposition: SearchDisposition,
    sections: Option<Vec<MarkdownSection>>,
}

#[derive(Debug)]
struct MarkdownSection {
    key: String,
    page_title: String,
    heading_ancestry: Vec<String>,
    heading: Option<String>,
    body: String,
}

pub(crate) fn search<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let options = parse_search_options(args)?;
    let root = current_tree_root(env)?;
    let tree = read_working_tree_state(&root)?;
    let folders = select_readable_folders(&tree, &options.folders)?;
    reconcile_search_indexes(&root)?;
    let mut evidence = Vec::new();
    for folder in &folders {
        let path = folder_index_path(&root, folder);
        let mut connection = open_folder_index(&path)?;
        let _ = initialize_index_schema(&connection, folder)?;
        evidence.extend(search_folder_index(
            &mut connection,
            folder,
            &options.query,
            MAX_SEARCH_LIMIT,
        )?);
    }
    evidence.sort_by(|left, right| {
        right
            .normalized_bm25
            .total_cmp(&left.normalized_bm25)
            .then(left.raw_bm25.total_cmp(&right.raw_bm25))
            .then(left.folder_id.cmp(&right.folder_id))
            .then(left.source_brain_id.cmp(&right.source_brain_id))
            .then(left.page_path.cmp(&right.page_path))
            .then(left.heading_ancestry.cmp(&right.heading_ancestry))
    });
    evidence.truncate(options.limit);
    for (index, result) in evidence.iter_mut().enumerate() {
        result.rank = index + 1;
    }
    let report = SearchReport {
        query: options.query,
        mode: "lexical",
        searched_folders: folders
            .iter()
            .map(|folder| {
                folder.source_brain_id.as_ref().map_or_else(
                    || folder.folder_id.clone(),
                    |source| format!("{source}:{}", folder.folder_id),
                )
            })
            .collect(),
        results: evidence,
    };
    if json {
        write_json(output, &report)
    } else {
        write_search_report(output, &report)
    }
}

fn parse_search_options(args: &[String]) -> Result<SearchOptions, CliError> {
    let mut query = None;
    let mut folders = Vec::new();
    let mut limit = DEFAULT_SEARCH_LIMIT;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--lexical-only" => index += 1,
            "--folder" => {
                let value = required_search_option(args, index, "--folder")?;
                folders.push(value.to_owned());
                index += 2;
            }
            "--limit" => {
                limit = parse_limit(required_search_option(args, index, "--limit")?)?;
                index += 2;
            }
            value if value.starts_with("--folder=") => {
                let value = value.trim_start_matches("--folder=");
                if value.is_empty() {
                    return Err(CliError::MissingArgument("--folder"));
                }
                folders.push(value.to_owned());
                index += 1;
            }
            value if value.starts_with("--limit=") => {
                limit = parse_limit(value.trim_start_matches("--limit="))?;
                index += 1;
            }
            value if value.starts_with("--") => {
                return Err(CliError::InvalidInput(format!(
                    "unknown search option {value}"
                )));
            }
            value => {
                if query.replace(value.to_owned()).is_some() {
                    return Err(CliError::InvalidInput(
                        "search query must be supplied as one quoted argument".to_owned(),
                    ));
                }
                index += 1;
            }
        }
    }
    let query = query.ok_or(CliError::MissingArgument("query"))?;
    if fts_query(&query).is_none() {
        return Err(CliError::InvalidInput(
            "search query must contain at least one letter or number".to_owned(),
        ));
    }
    Ok(SearchOptions {
        query,
        folders,
        limit,
    })
}

fn required_search_option<'a>(
    args: &'a [String],
    index: usize,
    name: &'static str,
) -> Result<&'a str, CliError> {
    args.get(index + 1)
        .filter(|value| !value.starts_with("--"))
        .map(String::as_str)
        .ok_or(CliError::MissingArgument(name))
}

fn parse_limit(value: &str) -> Result<usize, CliError> {
    let limit = value.parse::<usize>().map_err(|_| {
        CliError::InvalidInput("--limit must be an integer from 1 to 50".to_owned())
    })?;
    if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err(CliError::InvalidInput(
            "--limit must be an integer from 1 to 50".to_owned(),
        ));
    }
    Ok(limit)
}

fn select_readable_folders<'a>(
    tree: &'a BrainWorkingTreeStateManifest,
    requested: &[String],
) -> Result<Vec<&'a WorkingTreeFolderRoot>, CliError> {
    let mut readable = tree
        .folder_roots
        .iter()
        .filter(|folder| folder.can_read && !folder.metadata_only)
        .collect::<Vec<_>>();
    readable.sort_by(|left, right| left.folder_id.cmp(&right.folder_id));
    if requested.is_empty() {
        return Ok(readable);
    }
    let requested = requested.iter().collect::<BTreeSet<_>>();
    let mut selected = Vec::new();
    for request in requested {
        let matches = readable
            .iter()
            .copied()
            .filter(|folder| folder_matches_selector(folder, request))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(CliError::InvalidInput(format!(
                "Folder {request} is unknown or not readable"
            )));
        }
        if matches.len() > 1 {
            return Err(CliError::InvalidInput(format!(
                "Folder selector {request} is ambiguous; use <source-brain-id>:<folder-id>"
            )));
        }
        if !selected.iter().any(|folder: &&WorkingTreeFolderRoot| {
            folder.folder_id == matches[0].folder_id
                && folder.source_brain_id == matches[0].source_brain_id
        }) {
            selected.push(matches[0]);
        }
    }
    selected.sort_by(|left, right| {
        left.source_brain_id
            .cmp(&right.source_brain_id)
            .then(left.folder_id.cmp(&right.folder_id))
    });
    Ok(selected)
}

fn folder_matches_selector(folder: &WorkingTreeFolderRoot, selector: &str) -> bool {
    folder.folder_id == selector
        || folder.path == selector
        || folder
            .source_brain_id
            .as_deref()
            .is_some_and(|source| selector == format!("{source}:{}", folder.folder_id))
}

pub(crate) fn reconcile_search_indexes(root: &Path) -> Result<usize, CliError> {
    let tree = read_working_tree_state(root)?;
    let agent = read_agent_state(root)?;
    let mut changed = 0;
    for folder in tree
        .folder_roots
        .iter()
        .filter(|folder| folder.can_read && !folder.metadata_only)
    {
        let (_, folder_changed) = reconcile_folder_index_with_count(root, &tree, &agent, folder)?;
        changed += folder_changed;
    }
    remove_unreadable_folder_indexes(root, &tree)?;
    Ok(changed)
}

/// Apply one successful sync report to existing indexes without walking every Folder.
/// A missing index falls back to one full Folder build; subsequent hot updates touch
/// only the Pages named by the sync engine.
pub(crate) fn reconcile_search_changes(
    root: &Path,
    report: &SyncOnceReport,
) -> Result<usize, CliError> {
    let tree = read_working_tree_state(root)?;
    let agent = read_agent_state(root)?;
    let mut affected = BTreeMap::<(Option<String>, String), BTreeSet<String>>::new();
    for change in report
        .local_changes
        .iter()
        .chain(&report.remote_changes)
        .chain(&report.conflicts)
    {
        collect_affected_page(&tree, change, &mut affected)?;
    }

    let mut changed = 0;
    for ((source_brain_id, folder_id), paths) in affected {
        let folder = tree
            .folder_roots
            .iter()
            .find(|folder| {
                folder.folder_id == folder_id && folder.source_brain_id == source_brain_id
            })
            .ok_or_else(|| {
                CliError::SearchIndex(format!("sync report referenced unknown Folder {folder_id}"))
            })?;
        if !folder.can_read || folder.metadata_only {
            continue;
        }
        let index_path = folder_index_path(root, folder);
        if !index_path.exists() || paths.is_empty() {
            changed += reconcile_folder_index_with_count(root, &tree, &agent, folder)?.1;
        } else {
            changed += reconcile_folder_paths(root, &tree, &agent, folder, &paths)?;
        }
    }
    for folder in tree
        .folder_roots
        .iter()
        .filter(|folder| folder.can_read && !folder.metadata_only)
    {
        if !folder_index_path(root, folder).exists() {
            changed += reconcile_folder_index_with_count(root, &tree, &agent, folder)?.1;
        }
    }
    remove_unreadable_folder_indexes(root, &tree)?;
    Ok(changed)
}

pub(crate) fn reconcile_local_search_paths(
    root: &Path,
    paths: &[String],
) -> Result<usize, CliError> {
    let report = SyncOnceReport {
        status: "local".to_owned(),
        latest_sequence: 0,
        record_count: paths.len(),
        server_url: String::new(),
        local_changes: paths
            .iter()
            .map(|path| SyncChangeReport {
                status: "local".to_owned(),
                action: "update".to_owned(),
                actor_npub: None,
                sequence: None,
                path: Some(path.clone()),
                from_path: None,
                folder_id: None,
                source_brain_id: None,
                object_id: None,
                route: "working-tree".to_owned(),
                reason: None,
            })
            .collect(),
        remote_changes: Vec::new(),
        conflicts: Vec::new(),
    };
    reconcile_search_changes(root, &report)
}

fn collect_affected_page(
    tree: &BrainWorkingTreeStateManifest,
    change: &SyncChangeReport,
    affected: &mut BTreeMap<(Option<String>, String), BTreeSet<String>>,
) -> Result<(), CliError> {
    let candidate_paths = [change.path.as_deref(), change.from_path.as_deref()];
    let folders = tree
        .folder_roots
        .iter()
        .filter(|folder| {
            change
                .folder_id
                .as_deref()
                .is_none_or(|folder_id| folder.folder_id == folder_id)
                && change
                    .source_brain_id
                    .as_deref()
                    .is_none_or(|source| folder.source_brain_id.as_deref() == Some(source))
                && candidate_paths
                    .iter()
                    .flatten()
                    .any(|path| relative_page_path(&folder.path, path).is_some())
        })
        .collect::<Vec<_>>();
    if folders.len() > 1 {
        return Err(CliError::SearchIndex(format!(
            "sync report path is ambiguous across {} Folders",
            folders.len()
        )));
    }
    let Some(folder) = folders.first() else {
        return Ok(());
    };
    let paths = affected
        .entry((folder.source_brain_id.clone(), folder.folder_id.clone()))
        .or_default();
    for path in candidate_paths.into_iter().flatten() {
        if let Some(relative) = relative_page_path(&folder.path, path) {
            paths.insert(relative);
        }
    }
    Ok(())
}

fn relative_page_path(folder_path: &str, report_path: &str) -> Option<String> {
    let relative = Path::new(report_path)
        .strip_prefix(Path::new(folder_path))
        .ok()?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    relative.to_str().map(|value| value.replace('\\', "/"))
}

fn reconcile_folder_paths(
    root: &Path,
    tree: &BrainWorkingTreeStateManifest,
    agent: &AgentState,
    folder: &WorkingTreeFolderRoot,
    paths: &BTreeSet<String>,
) -> Result<usize, CliError> {
    let (mut connection, rebuilt) = open_or_rebuild_folder_index(root, folder)?;
    if rebuilt {
        drop(connection);
        return reconcile_folder_index_with_count(root, tree, agent, folder)
            .map(|(_, changed)| changed);
    }
    let folder_root = validated_folder_root(root, &folder.path)?;
    let manifests = tree
        .objects
        .iter()
        .filter(|object| {
            object.folder_id == folder.folder_id
                && object.source_brain_id == folder.source_brain_id
                && object.content_type.starts_with("text/markdown")
        })
        .map(|object| (object.path.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    let transaction = connection.transaction().map_err(search_index_error)?;
    let mut changed = 0;
    for relative in paths {
        let path = safe_page_path(&folder_root, relative)?;
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            changed += delete_indexed_page(&transaction, relative)?;
            continue;
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || !is_indexable_markdown(&folder_root, &path)
        {
            changed += delete_indexed_page(&transaction, relative)?;
            continue;
        }
        if metadata.len() > MAX_INDEXED_MARKDOWN_BYTES {
            return Err(CliError::SearchIndex(format!(
                "Markdown Page {} exceeds the {} byte indexing limit",
                path.display(),
                MAX_INDEXED_MARKDOWN_BYTES
            )));
        }
        let markdown = fs::read_to_string(&path)?;
        let content_hash = sha256_hex(markdown.as_bytes());
        let disposition = page_disposition(
            agent,
            folder,
            relative,
            manifests.get(relative.as_str()).copied(),
            &content_hash,
        );
        changed += reconcile_page_sections(
            &transaction,
            relative,
            &content_hash,
            disposition,
            &parse_markdown_sections(relative, &markdown),
        )?;
    }
    transaction.commit().map_err(search_index_error)?;
    Ok(changed)
}

fn safe_page_path(folder_root: &Path, relative: &str) -> Result<PathBuf, CliError> {
    let components = Path::new(relative)
        .components()
        .map(|component| {
            let Component::Normal(component) = component else {
                return Err(CliError::SearchIndex(format!(
                    "unsafe Page path in sync report: {relative}"
                )));
            };
            Ok(component.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut path = folder_root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        path.push(component);
        if index + 1 < components.len() {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(CliError::InsecureWorkingTree {
                        path,
                        reason: "search update requires real directories inside the Folder"
                            .to_owned(),
                    });
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(folder_root.join(relative));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(path)
}

fn reconcile_folder_index_with_count(
    root: &Path,
    tree: &BrainWorkingTreeStateManifest,
    agent: &AgentState,
    folder: &WorkingTreeFolderRoot,
) -> Result<(Connection, usize), CliError> {
    let (mut connection, _) = open_or_rebuild_folder_index(root, folder)?;
    let known = known_page_hashes(&connection)?;
    let pages = collect_folder_pages(root, tree, agent, folder, &known)?;
    let current_paths = pages
        .iter()
        .map(|page| page.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut changed = 0;
    let transaction = connection.transaction().map_err(search_index_error)?;
    for stale in known
        .keys()
        .filter(|path| !current_paths.contains(path.as_str()))
    {
        transaction
            .execute("DELETE FROM sections WHERE page_path = ?1", [stale])
            .map_err(search_index_error)?;
        transaction
            .execute("DELETE FROM pages WHERE path = ?1", [stale])
            .map_err(search_index_error)?;
        changed += 1;
    }
    for page in pages {
        let disposition = disposition_name(page.disposition);
        let Some(sections) = page.sections else {
            continue;
        };
        let known_sections = {
            let mut statement = transaction
                .prepare("SELECT section_key, section_hash FROM sections WHERE page_path = ?1")
                .map_err(search_index_error)?;
            let rows = statement
                .query_map([&page.path], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(search_index_error)?;
            rows.collect::<Result<BTreeMap<String, String>, _>>()
                .map_err(search_index_error)?
        };
        let current_section_keys = sections
            .iter()
            .map(|section| section.key.as_str())
            .collect::<BTreeSet<_>>();
        for stale_key in known_sections
            .keys()
            .filter(|key| !current_section_keys.contains(key.as_str()))
        {
            transaction
                .execute(
                    "DELETE FROM sections WHERE page_path = ?1 AND section_key = ?2",
                    params![&page.path, stale_key],
                )
                .map_err(search_index_error)?;
            changed += 1;
        }
        for section in &sections {
            let heading_ancestry = serde_json::to_string(&section.heading_ancestry)?;
            let section_hash = section_fingerprint(section, disposition);
            if known_sections
                .get(&section.key)
                .is_some_and(|known_hash| known_hash == &section_hash)
            {
                continue;
            }
            transaction
                .execute(
                    "DELETE FROM sections WHERE page_path = ?1 AND section_key = ?2",
                    params![&page.path, &section.key],
                )
                .map_err(search_index_error)?;
            transaction
                .execute(
                    "INSERT INTO sections (section_key, page_path, page_location, page_title, heading_ancestry, heading, body, disposition, section_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        &section.key,
                        &page.path,
                        &page.path,
                        &section.page_title,
                        heading_ancestry,
                        &section.heading,
                        &section.body,
                        disposition,
                        section_hash,
                    ],
                )
                .map_err(search_index_error)?;
            changed += 1;
        }
        transaction
            .execute(
                "INSERT INTO pages (path, content_hash, disposition) VALUES (?1, ?2, ?3) ON CONFLICT(path) DO UPDATE SET content_hash = excluded.content_hash, disposition = excluded.disposition",
                params![&page.path, &page.content_hash, disposition],
            )
            .map_err(search_index_error)?;
    }
    transaction.commit().map_err(search_index_error)?;
    Ok((connection, changed))
}

fn open_or_rebuild_folder_index(
    root: &Path,
    folder: &WorkingTreeFolderRoot,
) -> Result<(Connection, bool), CliError> {
    let path = folder_index_path(root, folder);
    let connection = match open_initialized_folder_index(&path, folder) {
        Ok((connection, schema_reset)) => (connection, schema_reset),
        Err(CliError::SearchIndexCorrupt(_))
            if fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink()) =>
        {
            remove_folder_index_directory(
                path.parent()
                    .ok_or_else(|| CliError::SearchIndex("index path has no parent".to_owned()))?,
            )?;
            let (connection, _) = open_initialized_folder_index(&path, folder)?;
            (connection, true)
        }
        Err(error) => return Err(error),
    };
    Ok(connection)
}

fn open_initialized_folder_index(
    path: &Path,
    folder: &WorkingTreeFolderRoot,
) -> Result<(Connection, bool), CliError> {
    let connection = open_folder_index(path)?;
    let integrity: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(search_index_error)?;
    if integrity != "ok" {
        return Err(CliError::SearchIndexCorrupt(integrity));
    }
    let schema_reset = initialize_index_schema(&connection, folder)?;
    Ok((connection, schema_reset))
}

fn open_folder_index(path: &Path) -> Result<Connection, CliError> {
    let parent = path.parent().ok_or_else(|| {
        CliError::SearchIndex("Folder index path has no parent directory".to_owned())
    })?;
    create_private_directory_if_missing(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(CliError::SearchIndex(format!(
            "refusing non-file Folder index at {}",
            path.display()
        )));
    }
    let connection = Connection::open(path).map_err(search_index_error)?;
    // The index is derived and always rebuildable. An in-memory rollback journal
    // preserves atomic transactions without writing section plaintext to sidecars.
    connection
        .pragma_update(None, "journal_mode", "MEMORY")
        .map_err(search_index_error)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(search_index_error)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(search_index_error)?;
    set_private_file_permissions(path)?;
    Ok(connection)
}

fn initialize_index_schema(
    connection: &Connection,
    folder: &WorkingTreeFolderRoot,
) -> Result<bool, CliError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .map_err(search_index_error)?;
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(search_index_error)?;
    let schema_is_current = version.as_deref() == Some(SEARCH_INDEX_VERSION)
        && table_columns(connection, "pages")? == ["path", "content_hash", "disposition"]
        && table_columns(connection, "sections")?
            == [
                "section_key",
                "page_path",
                "page_location",
                "page_title",
                "heading_ancestry",
                "heading",
                "body",
                "disposition",
                "section_hash",
            ];
    if !schema_is_current {
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS sections; DROP TABLE IF EXISTS pages; DELETE FROM metadata;",
            )
            .map_err(search_index_error)?;
    }
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS pages (path TEXT PRIMARY KEY, content_hash TEXT NOT NULL, disposition TEXT NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS sections USING fts5(
                section_key UNINDEXED, page_path UNINDEXED, page_location,
                page_title, heading_ancestry, heading, body, disposition UNINDEXED,
                section_hash UNINDEXED, tokenize = 'unicode61 remove_diacritics 2'
             );",
        )
        .map_err(search_index_error)?;
    for (key, value) in [
        ("version", SEARCH_INDEX_VERSION),
        ("folder_id", folder.folder_id.as_str()),
        ("folder_path", folder.path.as_str()),
        (
            "source_brain_id",
            folder.source_brain_id.as_deref().unwrap_or(""),
        ),
    ] {
        connection
            .execute(
                "INSERT INTO metadata (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(search_index_error)?;
    }
    Ok(!schema_is_current)
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, CliError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info('{table}')"))
        .map_err(search_index_error)?;
    let rows = statement
        .query_map([], |row| row.get(1))
        .map_err(search_index_error)?;
    rows.collect::<Result<_, _>>().map_err(search_index_error)
}

fn known_page_hashes(
    connection: &Connection,
) -> Result<BTreeMap<String, (String, String)>, CliError> {
    let mut statement = connection
        .prepare("SELECT path, content_hash, disposition FROM pages")
        .map_err(search_index_error)?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))
        .map_err(search_index_error)?;
    rows.collect::<Result<_, _>>().map_err(search_index_error)
}

fn delete_indexed_page(
    transaction: &rusqlite::Transaction<'_>,
    page_path: &str,
) -> Result<usize, CliError> {
    let sections = transaction
        .execute("DELETE FROM sections WHERE page_path = ?1", [page_path])
        .map_err(search_index_error)?;
    let pages = transaction
        .execute("DELETE FROM pages WHERE path = ?1", [page_path])
        .map_err(search_index_error)?;
    Ok(usize::from(sections > 0 || pages > 0))
}

fn reconcile_page_sections(
    transaction: &rusqlite::Transaction<'_>,
    page_path: &str,
    content_hash: &str,
    disposition: SearchDisposition,
    sections: &[MarkdownSection],
) -> Result<usize, CliError> {
    let disposition = disposition_name(disposition);
    let known_sections = {
        let mut statement = transaction
            .prepare("SELECT section_key, section_hash FROM sections WHERE page_path = ?1")
            .map_err(search_index_error)?;
        let rows = statement
            .query_map([page_path], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(search_index_error)?;
        rows.collect::<Result<BTreeMap<String, String>, _>>()
            .map_err(search_index_error)?
    };
    let current_keys = sections
        .iter()
        .map(|section| section.key.as_str())
        .collect::<BTreeSet<_>>();
    let mut changed = 0;
    for stale in known_sections
        .keys()
        .filter(|key| !current_keys.contains(key.as_str()))
    {
        transaction
            .execute(
                "DELETE FROM sections WHERE page_path = ?1 AND section_key = ?2",
                params![page_path, stale],
            )
            .map_err(search_index_error)?;
        changed += 1;
    }
    for section in sections {
        let heading_ancestry = serde_json::to_string(&section.heading_ancestry)?;
        let section_hash = section_fingerprint(section, disposition);
        if known_sections
            .get(&section.key)
            .is_some_and(|known_hash| known_hash == &section_hash)
        {
            continue;
        }
        transaction
            .execute(
                "DELETE FROM sections WHERE page_path = ?1 AND section_key = ?2",
                params![page_path, &section.key],
            )
            .map_err(search_index_error)?;
        transaction
            .execute(
                "INSERT INTO sections (section_key, page_path, page_location, page_title, heading_ancestry, heading, body, disposition, section_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    &section.key,
                    page_path,
                    page_path,
                    &section.page_title,
                    heading_ancestry,
                    &section.heading,
                    &section.body,
                    disposition,
                    section_hash,
                ],
            )
            .map_err(search_index_error)?;
        changed += 1;
    }
    transaction
        .execute(
            "INSERT INTO pages (path, content_hash, disposition) VALUES (?1, ?2, ?3) ON CONFLICT(path) DO UPDATE SET content_hash = excluded.content_hash, disposition = excluded.disposition",
            params![page_path, content_hash, disposition],
        )
        .map_err(search_index_error)?;
    Ok(changed)
}

fn collect_folder_pages(
    root: &Path,
    tree: &BrainWorkingTreeStateManifest,
    agent: &AgentState,
    folder: &WorkingTreeFolderRoot,
    known: &BTreeMap<String, (String, String)>,
) -> Result<Vec<IndexedPage>, CliError> {
    let folder_root = validated_folder_root(root, &folder.path)?;
    let mut pending = vec![folder_root.clone()];
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if !matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("_wiki" | ".finitebrain")
                ) {
                    pending.push(path);
                }
            } else if is_indexable_markdown(&folder_root, &path) {
                if metadata.len() > MAX_INDEXED_MARKDOWN_BYTES {
                    return Err(CliError::SearchIndex(format!(
                        "Markdown Page {} exceeds the {} byte indexing limit",
                        path.display(),
                        MAX_INDEXED_MARKDOWN_BYTES
                    )));
                }
                paths.push(path);
                if paths.len() > MAX_INDEXED_FILES_PER_FOLDER {
                    return Err(CliError::SearchIndex(format!(
                        "Folder {} exceeds the {} file indexing limit",
                        folder.folder_id, MAX_INDEXED_FILES_PER_FOLDER
                    )));
                }
            }
        }
    }
    paths.sort();
    let manifests = tree
        .objects
        .iter()
        .filter(|object| {
            object.folder_id == folder.folder_id
                && object.source_brain_id == folder.source_brain_id
                && object.content_type.starts_with("text/markdown")
        })
        .map(|object| (object.path.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&folder_root)
                .map_err(|_| CliError::SearchIndex("Page escaped Folder root".to_owned()))?
                .to_str()
                .ok_or_else(|| CliError::SearchIndex("Page path is not UTF-8".to_owned()))?
                .replace('\\', "/");
            let markdown = fs::read_to_string(&path)?;
            let content_hash = sha256_hex(markdown.as_bytes());
            let disposition = page_disposition(
                agent,
                folder,
                &relative,
                manifests.get(relative.as_str()).copied(),
                &content_hash,
            );
            let disposition_name = disposition_name(disposition);
            let unchanged = known
                .get(&relative)
                .is_some_and(|(known_hash, known_disposition)| {
                    known_hash == &content_hash && known_disposition == disposition_name
                });
            Ok(IndexedPage {
                path: relative.clone(),
                content_hash,
                disposition,
                sections: (!unchanged).then(|| parse_markdown_sections(&relative, &markdown)),
            })
        })
        .collect()
}

fn validated_folder_root(root: &Path, value: &str) -> Result<PathBuf, CliError> {
    let relative = Path::new(value);
    if relative.as_os_str().is_empty() {
        return Err(CliError::InvalidInput(
            "working-tree Folder path cannot be empty".to_owned(),
        ));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(CliError::InvalidInput(format!(
                "working-tree Folder path is not a safe relative path: {value}"
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CliError::InsecureWorkingTree {
                path: current,
                reason: "search traversal requires real directories inside the Working Tree"
                    .to_owned(),
            });
        }
    }
    Ok(current)
}

fn is_indexable_markdown(folder_root: &Path, path: &Path) -> bool {
    if path.extension().and_then(|value| value.to_str()) != Some("md") {
        return false;
    }
    let Ok(relative) = path.strip_prefix(folder_root) else {
        return false;
    };
    !matches!(
        relative.file_name().and_then(|name| name.to_str()),
        Some("AGENTS.md" | "_index.md")
    )
}

fn page_disposition(
    agent: &AgentState,
    folder: &WorkingTreeFolderRoot,
    page_path: &str,
    object: Option<&WorkingTreeObjectManifestEntry>,
    content_hash: &str,
) -> SearchDisposition {
    if agent.conflicts.iter().any(|conflict| {
        conflict.state == ConflictState::Open
            && conflict.folder_id.as_deref() == Some(folder.folder_id.as_str())
            && conflict.path.as_deref().is_some_and(|path| {
                path == page_path || path == format!("{}/{}", folder.path, page_path)
            })
    }) {
        SearchDisposition::Conflicted
    } else if object.is_some_and(|object| object.content_hash == content_hash) {
        SearchDisposition::Synced
    } else {
        SearchDisposition::LocalOnly
    }
}

fn parse_markdown_sections(page_path: &str, markdown: &str) -> Vec<MarkdownSection> {
    let fallback_title = Path::new(page_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(page_path)
        .replace(['-', '_'], " ");
    let headings = commonmark_headings(markdown);
    let page_title = headings
        .iter()
        .find(|heading| heading.level == 1)
        .map(|heading| heading.title.clone())
        .unwrap_or(fallback_title);
    if headings.is_empty() {
        let mut sections = vec![MarkdownSection {
            key: String::new(),
            page_title,
            heading_ancestry: Vec::new(),
            heading: None,
            body: markdown.trim().to_owned(),
        }];
        assign_section_keys(&mut sections);
        return sections;
    }
    let mut ancestry = Vec::<String>::new();
    let mut sections = Vec::new();
    let preamble = markdown[..headings[0].start].trim();
    if !preamble.is_empty() {
        push_section(&mut sections, &page_title, &[], None, preamble);
    }
    for (index, heading) in headings.iter().enumerate() {
        ancestry.truncate(heading.level.saturating_sub(1));
        while ancestry.len() < heading.level.saturating_sub(1) {
            ancestry.push(String::new());
        }
        ancestry.push(heading.title.clone());
        let current_ancestry = ancestry
            .iter()
            .filter(|heading| !heading.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let body_end = headings
            .get(index + 1)
            .map(|next| next.start)
            .unwrap_or(markdown.len());
        push_section(
            &mut sections,
            &page_title,
            &current_ancestry,
            Some(&heading.title),
            markdown[heading.end..body_end].trim(),
        );
    }
    assign_section_keys(&mut sections);
    sections
}

#[derive(Debug)]
struct CommonMarkHeading {
    level: usize,
    title: String,
    start: usize,
    end: usize,
}

fn commonmark_headings(markdown: &str) -> Vec<CommonMarkHeading> {
    let mut headings = Vec::new();
    let mut current = None::<CommonMarkHeading>;
    for (event, range) in Parser::new_ext(markdown, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current = Some(CommonMarkHeading {
                    level: heading_level(level),
                    title: String::new(),
                    start: range.start,
                    end: range.end,
                });
            }
            Event::Text(text) | Event::Code(text) if current.is_some() => current
                .as_mut()
                .expect("heading is present")
                .title
                .push_str(&text),
            Event::SoftBreak | Event::HardBreak if current.is_some() => current
                .as_mut()
                .expect("heading is present")
                .title
                .push(' '),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(mut heading) = current.take() {
                    heading.title = heading.title.trim().to_owned();
                    heading.end = range.end;
                    if !heading.title.is_empty() {
                        headings.push(heading);
                    }
                }
            }
            _ => {}
        }
    }
    headings
}

fn heading_level(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn push_section(
    sections: &mut Vec<MarkdownSection>,
    page_title: &str,
    ancestry: &[String],
    heading: Option<&str>,
    body: &str,
) {
    let chunks = split_section_body(body);
    if chunks.is_empty() && heading.is_some() {
        sections.push(MarkdownSection {
            key: String::new(),
            page_title: page_title.to_owned(),
            heading_ancestry: ancestry.to_vec(),
            heading: heading.map(str::to_owned),
            body: String::new(),
        });
    }
    for body in chunks {
        sections.push(MarkdownSection {
            key: String::new(),
            page_title: page_title.to_owned(),
            heading_ancestry: ancestry.to_vec(),
            heading: heading.map(str::to_owned),
            body,
        });
    }
}

fn split_section_body(body: &str) -> Vec<String> {
    let characters = body.trim().chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return Vec::new();
    }
    if characters.len() <= MAX_SECTION_CHARS {
        return vec![characters.into_iter().collect()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < characters.len() {
        let hard_end = (start + MAX_SECTION_CHARS).min(characters.len());
        let end = if hard_end == characters.len() {
            hard_end
        } else {
            paragraph_boundary(&characters, start, hard_end).unwrap_or(hard_end)
        };
        let chunk = characters[start..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_owned();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        if end == characters.len() {
            break;
        }
        let overlapped_start = end.saturating_sub(SECTION_OVERLAP_CHARS);
        start = if overlapped_start > start {
            overlapped_start
        } else {
            end
        };
    }
    chunks
}

fn paragraph_boundary(characters: &[char], start: usize, hard_end: usize) -> Option<usize> {
    let minimum = start + MAX_SECTION_CHARS / 2;
    (minimum..hard_end.saturating_sub(1))
        .rev()
        .find(|index| characters[*index] == '\n' && characters[*index + 1] == '\n')
}

fn assign_section_keys(sections: &mut [MarkdownSection]) {
    let mut occurrences = BTreeMap::<String, usize>::new();
    for section in sections {
        let identity = serde_json::to_string(&(
            section.heading_ancestry.as_slice(),
            section.heading.as_deref(),
        ))
        .expect("Markdown Section identity is serializable");
        let occurrence = occurrences.entry(identity.clone()).or_default();
        section.key = format!("{}:{occurrence}", sha256_hex(identity.as_bytes()));
        *occurrence += 1;
    }
}

fn section_fingerprint(section: &MarkdownSection, disposition: &str) -> String {
    let mut hasher = Sha256::new();
    for value in [
        section.page_title.as_str(),
        section.heading.as_deref().unwrap_or(""),
        section.body.as_str(),
        disposition,
    ] {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }
    for heading in &section.heading_ancestry {
        hasher.update(heading.len().to_le_bytes());
        hasher.update(heading.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn search_folder_index(
    connection: &mut Connection,
    folder: &WorkingTreeFolderRoot,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchEvidence>, CliError> {
    let fts_query = fts_query(query).expect("validated search query");
    let mut statement = connection
        .prepare(
            "SELECT page_path, page_title, heading_ancestry, heading,
                    snippet(sections, 6, '', '', ' … ', 24), disposition,
                    bm25(sections, 0.0, 0.0, 3.0, 4.0, 2.0, 2.0, 1.0, 0.0, 0.0)
             FROM sections WHERE sections MATCH ?1
             ORDER BY bm25(sections, 0.0, 0.0, 3.0, 4.0, 2.0, 2.0, 1.0, 0.0, 0.0)
             LIMIT ?2",
        )
        .map_err(search_index_error)?;
    let rows = statement
        .query_map(params![fts_query, limit as i64], |row| {
            let ancestry_json: String = row.get(2)?;
            let disposition: String = row.get(5)?;
            Ok(SearchEvidence {
                rank: 0,
                folder_id: folder.folder_id.clone(),
                source_brain_id: folder.source_brain_id.clone(),
                folder_path: folder.path.clone(),
                page_path: row.get(0)?,
                page_title: row.get(1)?,
                heading_ancestry: serde_json::from_str(&ancestry_json).unwrap_or_default(),
                heading: row.get(3)?,
                excerpt: row.get(4)?,
                disposition: parse_disposition(&disposition),
                signals: ["lexical"],
                normalized_bm25: 0.0,
                raw_bm25: row.get(6)?,
            })
        })
        .map_err(search_index_error)?;
    let mut evidence = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(search_index_error)?;
    let strongest = evidence
        .iter()
        .map(|result| (-result.raw_bm25).max(0.0))
        .fold(0.0_f64, f64::max);
    if strongest > 0.0 {
        for result in &mut evidence {
            result.normalized_bm25 = (-result.raw_bm25).max(0.0) / strongest;
        }
    }
    Ok(evidence)
}

fn fts_query(query: &str) -> Option<String> {
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn write_search_report<W: Write>(output: &mut W, report: &SearchReport) -> Result<(), CliError> {
    writeln!(
        output,
        "search {} folder(s), {} result(s)",
        report.searched_folders.len(),
        report.results.len()
    )?;
    for result in &report.results {
        let heading = result
            .heading_ancestry
            .last()
            .map(|heading| format!(" > {heading}"))
            .unwrap_or_default();
        writeln!(
            output,
            "{}. {}/{}{} [{}; lexical]",
            result.rank,
            result.folder_path,
            result.page_path,
            heading,
            disposition_name(result.disposition)
        )?;
        writeln!(output, "   {}", result.excerpt.replace('\n', " "))?;
    }
    Ok(())
}

fn folder_index_path(root: &Path, folder: &WorkingTreeFolderRoot) -> PathBuf {
    let identity = format!(
        "{}\n{}",
        folder.source_brain_id.as_deref().unwrap_or(""),
        folder.folder_id
    );
    root.join(".finitebrain/search-indexes")
        .join(sha256_hex(identity.as_bytes()))
        .join("index.sqlite3")
}

fn remove_unreadable_folder_indexes(
    root: &Path,
    tree: &BrainWorkingTreeStateManifest,
) -> Result<(), CliError> {
    let directory = root.join(".finitebrain/search-indexes");
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(());
    };
    let retained = tree
        .folder_roots
        .iter()
        .filter(|folder| folder.can_read && !folder.metadata_only)
        .filter_map(|folder| {
            folder_index_path(root, folder)
                .parent()
                .map(Path::to_path_buf)
        })
        .collect::<BTreeSet<_>>();
    for entry in entries {
        let path = entry?.path();
        if !retained.contains(&path) {
            remove_search_index_entry(&path)?;
        }
    }
    Ok(())
}

fn remove_search_index_entry(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        return remove_folder_index_directory(path);
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if metadata.is_file() && !metadata.file_type().is_symlink() && is_legacy_index_filename(name) {
        fs::remove_file(path)?;
        return Ok(());
    }
    Err(CliError::SearchIndex(format!(
        "refusing unknown Folder index entry {}",
        path.display()
    )))
}

fn is_legacy_index_filename(name: &str) -> bool {
    let stem = name
        .strip_suffix(".sqlite3")
        .or_else(|| name.strip_suffix(".sqlite3-journal"))
        .or_else(|| name.strip_suffix(".sqlite3-wal"))
        .or_else(|| name.strip_suffix(".sqlite3-shm"));
    stem.is_some_and(|stem| stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn remove_folder_index_directory(directory: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::SearchIndex(format!(
            "refusing unsafe Folder index directory {}",
            directory.display()
        )));
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|name| name.to_str());
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || !matches!(
                name,
                Some(
                    "index.sqlite3"
                        | "index.sqlite3-journal"
                        | "index.sqlite3-wal"
                        | "index.sqlite3-shm"
                )
            )
        {
            return Err(CliError::SearchIndex(format!(
                "refusing unknown Folder index residue {}",
                path.display()
            )));
        }
        fs::remove_file(path)?;
    }
    fs::remove_dir(directory)?;
    Ok(())
}

fn sha256_hex(input: &[u8]) -> String {
    Sha256::digest(input)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn disposition_name(disposition: SearchDisposition) -> &'static str {
    match disposition {
        SearchDisposition::Synced => "synced",
        SearchDisposition::LocalOnly => "local_only",
        SearchDisposition::Conflicted => "conflicted",
    }
}

fn parse_disposition(value: &str) -> SearchDisposition {
    match value {
        "synced" => SearchDisposition::Synced,
        "conflicted" => SearchDisposition::Conflicted,
        _ => SearchDisposition::LocalOnly,
    }
}

fn search_index_error(error: rusqlite::Error) -> CliError {
    if error.sqlite_error_code().is_some_and(|code| {
        matches!(
            code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        )
    }) {
        CliError::SearchIndexCorrupt(error.to_string())
    } else {
        CliError::SearchIndex(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_sections_split_near_paragraphs_with_context_and_overlap() {
        let first = "a".repeat(MAX_SECTION_CHARS / 2);
        let second = "b".repeat(MAX_SECTION_CHARS / 2);
        let third = "c".repeat(MAX_SECTION_CHARS / 2);
        let markdown = format!("# Guide\n\n{first}\n\n{second}\n\n{third}\n");

        let sections = parse_markdown_sections("guide.md", &markdown);

        assert!(sections.len() > 1);
        assert!(
            sections
                .iter()
                .all(|section| section.body.chars().count() <= MAX_SECTION_CHARS)
        );
        assert!(sections.iter().all(|section| {
            section.page_title == "Guide"
                && section.heading_ancestry == ["Guide"]
                && section.heading.as_deref() == Some("Guide")
        }));
        assert!(sections[0].body.ends_with(&first));
        assert!(sections[1].body.starts_with('a'));
        assert!(sections.iter().any(|section| section.body.contains(&third)));
    }

    #[test]
    fn heading_only_pages_keep_their_heading_context() {
        let sections = parse_markdown_sections("outline.md", "# Outline\n\n## Empty topic\n");

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading.as_deref(), Some("Outline"));
        assert_eq!(sections[1].heading.as_deref(), Some("Empty topic"));
        assert_eq!(sections[1].heading_ancestry, ["Outline", "Empty topic"]);
    }

    #[test]
    fn commonmark_headings_ignore_code_and_preserve_setext_and_hash_titles() {
        let markdown = "# C#\n\n```md\n# Not a heading\n```\n\nIndented code:\n\n    # Also not a heading\n\nSetext topic\n------------\n\nUseful body.\n";

        let sections = parse_markdown_sections("languages.md", markdown);

        assert_eq!(sections[0].page_title, "C#");
        assert_eq!(sections[0].heading.as_deref(), Some("C#"));
        assert!(sections[0].body.contains("# Not a heading"));
        assert!(sections[0].body.contains("# Also not a heading"));
        assert_eq!(sections[1].heading.as_deref(), Some("Setext topic"));
        assert_eq!(sections[1].heading_ancestry, ["C#", "Setext topic"]);
    }

    #[test]
    fn duplicate_mounted_folder_ids_require_a_source_qualified_selector() {
        let tree = BrainWorkingTreeStateManifest {
            version: "test".to_owned(),
            folder_roots: vec![
                WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: Some("brain-a".to_owned()),
                    path: "Mounted/A".to_owned(),
                    can_read: true,
                    metadata_only: false,
                },
                WorkingTreeFolderRoot {
                    folder_id: "general".to_owned(),
                    source_brain_id: Some("brain-b".to_owned()),
                    path: "Mounted/B".to_owned(),
                    can_read: true,
                    metadata_only: false,
                },
            ],
            objects: Vec::new(),
            sync: finite_brain_core::portability::WorkingTreeSyncState { latest_sequence: 0 },
        };

        assert!(
            select_readable_folders(&tree, &["general".to_owned()])
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        let selected = select_readable_folders(&tree, &["brain-b:general".to_owned()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].source_brain_id.as_deref(), Some("brain-b"));
    }

    #[test]
    fn search_limit_accepts_the_public_range_only() {
        assert_eq!(parse_limit("1").unwrap(), 1);
        assert_eq!(parse_limit("50").unwrap(), 50);
        assert!(parse_limit("0").is_err());
        assert!(parse_limit("51").is_err());
    }
}
