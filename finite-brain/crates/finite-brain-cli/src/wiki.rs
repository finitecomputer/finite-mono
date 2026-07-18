use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(unix))]
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
#[cfg(unix)]
use std::{
    ffi::{CStr, OsStr},
    fs::File,
    os::{fd::OwnedFd, unix::ffi::OsStrExt},
};

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
#[cfg(unix)]
use rustix::fs::{Dir, Mode, OFlags, open, openat};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::{CliError, read_working_tree_state, validate_private_working_tree};

const MAX_WIKI_DEPTH: usize = 32;
const MAX_WIKI_ENTRIES: usize = 10_000;
const MAX_WIKI_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WIKI_TOTAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
struct WikiPage {
    folder_id: String,
    source_vault_id: Option<String>,
    display_path: String,
    page_path: String,
    title: String,
    body: String,
}

struct WikiScan<'a> {
    display_root: &'a Path,
    folder_id: &'a str,
    source_vault_id: Option<&'a str>,
    entry_count: &'a mut usize,
    body_bytes: &'a mut usize,
    pages: &'a mut Vec<WikiPage>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WikiLinkIssue {
    pub(crate) folder_id: String,
    pub(crate) path: String,
    pub(crate) reference: String,
    pub(crate) status: String,
    pub(crate) matches: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WikiLinkHealthReport {
    pub(crate) status: String,
    pub(crate) page_count: usize,
    pub(crate) resolved_link_count: usize,
    pub(crate) missing_link_count: usize,
    pub(crate) ambiguous_link_count: usize,
    pub(crate) issues: Vec<WikiLinkIssue>,
}

pub(crate) fn check_wiki_links(root: &Path) -> Result<WikiLinkHealthReport, CliError> {
    validate_private_working_tree(root)?;
    let state = read_working_tree_state(root)?;
    let mut pages = Vec::new();
    let mut entry_count = 0usize;
    let mut body_bytes = 0usize;
    #[cfg(unix)]
    let root_fd = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    for folder in state.folder_roots.iter().filter(|folder| folder.can_read) {
        let relative_root = safe_manifest_path(&folder.path)?;
        let mut scan = WikiScan {
            display_root: &relative_root,
            folder_id: &folder.folder_id,
            source_vault_id: folder.source_vault_id.as_deref(),
            entry_count: &mut entry_count,
            body_bytes: &mut body_bytes,
            pages: &mut pages,
        };
        #[cfg(unix)]
        {
            let folder_fd = open_folder_root(&root_fd, root, &relative_root)?;
            collect_wiki_pages_fd(&folder_fd, &relative_root, Path::new(""), 0, &mut scan)?;
        }
        #[cfg(not(unix))]
        {
            let folder_root = validate_folder_root(root, &relative_root)?;
            collect_wiki_pages(&folder_root, Path::new(""), 0, &mut scan)?;
        }
    }
    pages.sort_by(|left, right| left.display_path.cmp(&right.display_path));

    let mut resolved_link_count = 0usize;
    let mut issues = Vec::new();
    for source in &pages {
        for reference in extract_page_links(&source.body) {
            let matches = resolve_page_reference(&reference, &pages, source);
            if matches.len() == 1 {
                resolved_link_count += 1;
                continue;
            }
            issues.push(WikiLinkIssue {
                folder_id: source.folder_id.clone(),
                path: source.display_path.clone(),
                reference,
                status: if matches.is_empty() {
                    "missing".to_owned()
                } else {
                    "ambiguous".to_owned()
                },
                matches: matches
                    .into_iter()
                    .map(|page| page.display_path.clone())
                    .collect(),
            });
        }
    }
    let missing_link_count = issues
        .iter()
        .filter(|issue| issue.status == "missing")
        .count();
    let ambiguous_link_count = issues.len() - missing_link_count;
    Ok(WikiLinkHealthReport {
        status: if issues.is_empty() {
            "ok".to_owned()
        } else {
            "issues".to_owned()
        },
        page_count: pages.len(),
        resolved_link_count,
        missing_link_count,
        ambiguous_link_count,
        issues,
    })
}

#[cfg(unix)]
fn open_folder_root(
    root_fd: &OwnedFd,
    root: &Path,
    relative_root: &Path,
) -> Result<OwnedFd, CliError> {
    let mut current = rustix::io::dup(root_fd).map_err(std::io::Error::from)?;
    let mut display_path = root.to_path_buf();
    for component in relative_root.components() {
        let Component::Normal(component) = component else {
            return Err(CliError::InvalidInput(format!(
                "working-tree Folder path is not a safe relative path: {}",
                relative_root.display()
            )));
        };
        display_path.push(component);
        current = openat(
            &current,
            component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| CliError::InsecureWorkingTree {
            path: display_path.clone(),
            reason: format!("wiki traversal cannot safely open Folder root: {error}"),
        })?;
    }
    Ok(current)
}

#[cfg(unix)]
fn collect_wiki_pages_fd(
    directory_fd: &OwnedFd,
    directory_display: &Path,
    relative: &Path,
    depth: usize,
    scan: &mut WikiScan<'_>,
) -> Result<(), CliError> {
    if depth > MAX_WIKI_DEPTH {
        return Err(CliError::InsecureWorkingTree {
            path: directory_display.to_path_buf(),
            reason: "wiki traversal exceeds the supported depth".to_owned(),
        });
    }
    let remaining_entries = MAX_WIKI_ENTRIES.saturating_sub(*scan.entry_count);
    let mut names = Vec::new();
    let mut directory = Dir::read_from(directory_fd).map_err(std::io::Error::from)?;
    for entry in &mut directory {
        let entry = entry.map_err(std::io::Error::from)?;
        let name = entry.file_name();
        if name == c"." || name == c".." {
            continue;
        }
        names.push(name.to_owned());
        if names.len() > remaining_entries {
            return Err(CliError::InsecureWorkingTree {
                path: directory_display.to_path_buf(),
                reason: "wiki traversal exceeds the supported entry count".to_owned(),
            });
        }
    }
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));

    for name in names {
        *scan.entry_count += 1;
        let os_name = OsStr::from_bytes(name.to_bytes());
        let child_relative = relative.join(os_name);
        let child_display = directory_display.join(os_name);
        let child_fd = open_wiki_entry(directory_fd, &name, &child_display)?;
        let mut child = File::from(child_fd);
        let metadata = child.metadata()?;
        if metadata.is_dir() {
            if name.as_c_str() == c"_wiki" {
                continue;
            }
            let child_fd: OwnedFd = child.into();
            collect_wiki_pages_fd(&child_fd, &child_display, &child_relative, depth + 1, scan)?;
            continue;
        }
        if !metadata.is_file()
            || child_display
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("md")
            || child_relative == Path::new("AGENTS.md")
            || child_relative == Path::new("_index.md")
        {
            continue;
        }
        let body =
            read_bounded_wiki_body(&mut child, &child_display, metadata.len(), scan.body_bytes)?;
        push_wiki_page(&child_relative, body, scan);
    }
    Ok(())
}

#[cfg(unix)]
fn open_wiki_entry(
    directory_fd: &OwnedFd,
    name: &CStr,
    display_path: &Path,
) -> Result<OwnedFd, CliError> {
    openat(
        directory_fd,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| CliError::InsecureWorkingTree {
        path: display_path.to_path_buf(),
        reason: format!("wiki traversal refuses an unsafe entry: {error}"),
    })
}

#[cfg(not(unix))]
fn collect_wiki_pages(
    directory: &Path,
    relative: &Path,
    depth: usize,
    scan: &mut WikiScan<'_>,
) -> Result<(), CliError> {
    if depth > MAX_WIKI_DEPTH {
        return Err(CliError::InsecureWorkingTree {
            path: directory.to_path_buf(),
            reason: "wiki traversal exceeds the supported depth".to_owned(),
        });
    }
    let remaining_entries = MAX_WIKI_ENTRIES.saturating_sub(*scan.entry_count);
    let mut entries = fs::read_dir(directory)?
        .take(remaining_entries.saturating_add(1))
        .collect::<Result<Vec<_>, _>>()?;
    if entries.len() > remaining_entries {
        return Err(CliError::InsecureWorkingTree {
            path: directory.to_path_buf(),
            reason: "wiki traversal exceeds the supported entry count".to_owned(),
        });
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        *scan.entry_count += 1;
        if *scan.entry_count > MAX_WIKI_ENTRIES {
            return Err(CliError::InsecureWorkingTree {
                path: entry.path(),
                reason: "wiki traversal exceeds the supported entry count".to_owned(),
            });
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(CliError::InsecureWorkingTree {
                path,
                reason: "wiki traversal refuses symbolic links".to_owned(),
            });
        }
        let name = entry.file_name();
        let child_relative = relative.join(&name);
        if metadata.is_dir() {
            if name == "_wiki" {
                continue;
            }
            collect_wiki_pages(&path, &child_relative, depth + 1, scan)?;
            continue;
        }
        if !metadata.is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("md")
            || child_relative == Path::new("AGENTS.md")
            || child_relative == Path::new("_index.md")
        {
            continue;
        }
        let mut file = fs::File::open(&path)?;
        let body = read_bounded_wiki_body(&mut file, &path, metadata.len(), scan.body_bytes)?;
        push_wiki_page(&child_relative, body, scan);
    }
    Ok(())
}

fn resolve_page_reference<'a>(
    reference: &str,
    pages: &'a [WikiPage],
    source: &WikiPage,
) -> Vec<&'a WikiPage> {
    let mut matches = BTreeMap::new();
    for page in pages {
        if page_references(page).contains(reference) {
            matches.insert(page.display_path.clone(), page);
        }
    }
    let readable_matches = matches.into_values().collect::<Vec<_>>();
    let local_matches = readable_matches
        .iter()
        .copied()
        .filter(|page| {
            page.folder_id == source.folder_id && page.source_vault_id == source.source_vault_id
        })
        .collect::<Vec<_>>();
    if local_matches.is_empty() {
        readable_matches
    } else {
        local_matches
    }
}

fn page_references(page: &WikiPage) -> BTreeSet<String> {
    [
        page.title.as_str(),
        page.page_path.as_str(),
        page.page_path.rsplit('/').next().unwrap_or(&page.page_path),
    ]
    .into_iter()
    .map(normalize_page_reference)
    .filter(|reference| !reference.is_empty())
    .collect()
}

fn extract_page_links(markdown: &str) -> Vec<String> {
    let mut links = BTreeSet::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_WIKILINKS) {
        let Event::Start(Tag::Link {
            link_type,
            dest_url,
            ..
        }) = event
        else {
            continue;
        };
        let target = dest_url.split('#').next().unwrap_or_default().trim();
        let is_wiki_link = matches!(link_type, LinkType::WikiLink { .. });
        if target.is_empty()
            || (!is_wiki_link
                && (is_external_reference(target) || !is_markdown_page_destination(target)))
        {
            continue;
        }
        let target = normalize_page_reference(target);
        if !target.is_empty() {
            links.insert(target);
        }
    }
    links.into_iter().collect()
}

fn normalize_page_reference(value: &str) -> String {
    let mut reference = value.trim();
    if let Some(stripped) = reference.strip_prefix("./") {
        reference = stripped;
    }
    if let Some(stripped) = reference.strip_suffix(".md") {
        reference = stripped;
    }
    if let Some(stripped) = reference.strip_prefix('#') {
        reference = stripped;
    }
    reference.nfc().collect()
}

fn is_external_reference(value: &str) -> bool {
    if value.starts_with("//") {
        return true;
    }
    let Some(colon) = value.find(':') else {
        return false;
    };
    let scheme = &value[..colon];
    !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn is_markdown_page_destination(value: &str) -> bool {
    let filename = value.rsplit('/').next().unwrap_or(value);
    match filename.rsplit_once('.') {
        Some((_, extension)) => extension == "md",
        None => true,
    }
}

fn markdown_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

fn title_from_path(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_owned()
}

fn safe_manifest_path(value: &str) -> Result<PathBuf, CliError> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::InvalidInput(format!(
            "working-tree Folder path is not a safe relative path: {value}"
        )));
    }
    Ok(path)
}

#[cfg(not(unix))]
fn validate_folder_root(root: &Path, relative_root: &Path) -> Result<PathBuf, CliError> {
    let mut current = root.to_path_buf();
    for component in relative_root.components() {
        let Component::Normal(component) = component else {
            return Err(CliError::InvalidInput(format!(
                "working-tree Folder path is not a safe relative path: {}",
                relative_root.display()
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(CliError::InsecureWorkingTree {
                path: current,
                reason: "wiki traversal refuses symbolic links in Folder roots".to_owned(),
            });
        }
        if !metadata.is_dir() {
            return Err(CliError::InsecureWorkingTree {
                path: current,
                reason: "wiki Folder root component is not a directory".to_owned(),
            });
        }
    }
    Ok(current)
}

fn read_bounded_wiki_body<R: Read>(
    reader: &mut R,
    path: &Path,
    declared_size: u64,
    total_bytes: &mut usize,
) -> Result<String, CliError> {
    check_wiki_body_size(path, declared_size, *total_bytes)?;
    let available = MAX_WIKI_TOTAL_BYTES.saturating_sub(*total_bytes);
    let read_limit = MAX_WIKI_PAGE_BYTES.min(available).saturating_add(1);
    let mut body = String::new();
    reader.take(read_limit as u64).read_to_string(&mut body)?;
    *total_bytes = check_wiki_body_size(path, body.len() as u64, *total_bytes)?;
    Ok(body)
}

fn push_wiki_page(relative: &Path, body: String, scan: &mut WikiScan<'_>) {
    let page_path = slash_path(relative);
    let display_path = slash_path(&scan.display_root.join(relative));
    let title = markdown_title(&body).unwrap_or_else(|| title_from_path(&page_path));
    scan.pages.push(WikiPage {
        folder_id: scan.folder_id.to_owned(),
        source_vault_id: scan.source_vault_id.map(ToOwned::to_owned),
        display_path,
        page_path,
        title,
        body,
    });
}

fn check_wiki_body_size(path: &Path, size: u64, total_bytes: usize) -> Result<usize, CliError> {
    if size > MAX_WIKI_PAGE_BYTES as u64 {
        return Err(CliError::InvalidInput(format!(
            "wiki Page {} exceeds size limit {MAX_WIKI_PAGE_BYTES}",
            path.display()
        )));
    }
    let size = usize::try_from(size).map_err(|_| {
        CliError::InvalidInput(format!("wiki Page {} size is unsupported", path.display()))
    })?;
    let next_total = total_bytes
        .checked_add(size)
        .ok_or_else(|| CliError::InvalidInput("wiki Page scan size overflowed".to_owned()))?;
    if next_total > MAX_WIKI_TOTAL_BYTES {
        return Err(CliError::InvalidInput(format!(
            "wiki Page scan exceeds total size limit {MAX_WIKI_TOTAL_BYTES}"
        )));
    }
    Ok(next_total)
}

fn slash_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_body_limits_bound_each_page_and_the_whole_scan() {
        let path = Path::new("page.md");
        assert!(check_wiki_body_size(path, MAX_WIKI_PAGE_BYTES as u64, 0).is_ok());
        assert!(check_wiki_body_size(path, (MAX_WIKI_PAGE_BYTES + 1) as u64, 0).is_err());
        assert!(check_wiki_body_size(path, 1, MAX_WIKI_TOTAL_BYTES,).is_err());
    }

    #[test]
    fn reference_normalization_is_nfc_and_case_sensitive() {
        assert_eq!(normalize_page_reference("Guide.md"), "Guide");
        assert_eq!(normalize_page_reference("guide.md"), "guide");
        assert_eq!(
            normalize_page_reference("Re\u{301}sume\u{301}"),
            "R\u{e9}sum\u{e9}"
        );
    }

    #[test]
    fn commonmark_context_ignores_code_and_uses_first_reference_definition() {
        let markdown = "```md\n```not-a-close\n[Ghost](missing.md)\n```\n\n    [Ghost](missing.md)\n\nParagraph\n    [Paragraph Guide](paragraph.md)\n\n- item\n\n    [List Guide](list.md)\n\n[[Guide]] [Duplicate][duplicate]\n\n[Guide]: missing.md\n[duplicate]: upper.md\n[duplicate]: missing.md\n";
        assert_eq!(
            extract_page_links(markdown),
            vec![
                "Guide".to_owned(),
                "list".to_owned(),
                "paragraph".to_owned(),
                "upper".to_owned()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn opened_folder_handle_cannot_be_redirected_by_a_later_symlink_swap() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("vault");
        let external = tmp.path().join("external");
        std::fs::create_dir_all(root.join("Notes")).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(root.join("Notes/safe.md"), "# Safe\n").unwrap();
        std::fs::write(external.join("secret.md"), "# External Secret\n").unwrap();
        let root_fd = open(
            &root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap();
        let relative_root = PathBuf::from("Notes");
        let folder_fd = open_folder_root(&root_fd, &root, &relative_root).unwrap();
        std::fs::rename(root.join("Notes"), root.join("Original")).unwrap();
        symlink(&external, root.join("Notes")).unwrap();

        let mut entry_count = 0;
        let mut body_bytes = 0;
        let mut pages = Vec::new();
        let mut scan = WikiScan {
            display_root: &relative_root,
            folder_id: "notes",
            source_vault_id: None,
            entry_count: &mut entry_count,
            body_bytes: &mut body_bytes,
            pages: &mut pages,
        };
        collect_wiki_pages_fd(&folder_fd, &relative_root, Path::new(""), 0, &mut scan).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Safe");
    }
}
