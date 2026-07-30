use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::AgentdError;

const MAX_QUERY_CHARS: usize = 128;
const MAX_RESULTS: u16 = 40;
const MAX_MARKDOWN_SEARCH_BYTES: u64 = 512 * 1024;
const MAX_EXCERPT_CHARS: usize = 180;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSearchRequestV1 {
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSearchResultV1 {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ContextCatalog {
    workspace: PathBuf,
    skill_roots: Vec<SkillRoot>,
}

#[derive(Debug, Clone)]
struct SkillRoot {
    path: PathBuf,
    label: &'static str,
    priority: u8,
}

#[derive(Debug)]
struct RankedResult {
    score: u16,
    result: ContextSearchResultV1,
}

impl ContextCatalog {
    pub fn new(
        workspace: PathBuf,
        hermes_home: PathBuf,
        managed_skill_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let workspace = fs::canonicalize(&workspace).unwrap_or(workspace);
        let user_skills = hermes_home.join("skills");
        let mut skill_roots = vec![SkillRoot {
            path: fs::canonicalize(&user_skills).unwrap_or(user_skills),
            label: "User skill",
            priority: 0,
        }];
        skill_roots.extend(
            managed_skill_roots
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| SkillRoot {
                    path: fs::canonicalize(&path).unwrap_or(path),
                    label: "Managed skill",
                    priority: 1,
                }),
        );
        Self {
            workspace,
            skill_roots,
        }
    }

    pub fn search(
        &self,
        request: ContextSearchRequestV1,
        sites_json: Option<&str>,
    ) -> Result<Vec<ContextSearchResultV1>, AgentdError> {
        let query = request.query.trim();
        if query.chars().count() > MAX_QUERY_CHARS
            || request.limit == 0
            || request.limit > MAX_RESULTS
        {
            return Err(AgentdError::InvalidPayload(
                "Context search query or limit is invalid.".to_owned(),
            ));
        }
        let normalized = query.to_lowercase();
        let mut ranked = self.workspace_results(&normalized);
        ranked.extend(self.skill_results(&normalized));
        if let Some(sites_json) = sites_json {
            ranked.extend(site_results(sites_json, &normalized));
        }
        ranked.sort_by_key(|entry| {
            (
                Reverse(entry.score),
                Reverse(entry.result.updated_at_ms),
                entry.result.kind.clone(),
                entry.result.label.to_lowercase(),
            )
        });
        Ok(ranked
            .into_iter()
            .take(usize::from(request.limit))
            .map(|entry| entry.result)
            .collect())
    }

    fn workspace_results(&self, query: &str) -> Vec<RankedResult> {
        if !self.workspace.is_dir() {
            return Vec::new();
        }
        WalkDir::new(&self.workspace)
            .max_depth(12)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !hidden_entry(entry.path()))
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("md"))
            })
            .filter_map(|entry| self.workspace_result(entry.path(), query))
            .collect()
    }

    fn workspace_result(&self, path: &Path, query: &str) -> Option<RankedResult> {
        let relative = path.strip_prefix(&self.workspace).ok()?;
        if !safe_relative_path(relative) {
            return None;
        }
        let relative = relative.to_string_lossy().replace('\\', "/");
        let label = path.file_name()?.to_string_lossy().into_owned();
        let metadata = fs::symlink_metadata(path).ok()?;
        if metadata.file_type().is_symlink() {
            return None;
        }
        let updated_at_ms = modified_at_ms(&metadata);
        let content = (metadata.len() <= MAX_MARKDOWN_SEARCH_BYTES)
            .then(|| fs::read_to_string(path).ok())
            .flatten()
            .unwrap_or_default();
        let fingerprint = if metadata.len() <= MAX_MARKDOWN_SEARCH_BYTES {
            content_fingerprint(content.as_bytes())
        } else {
            format!(
                "mtime-size:{}:{}",
                modified_at_ms(&metadata),
                metadata.len()
            )
        };
        let score = lexical_score(&label, &relative, &content, query)?;
        Some(RankedResult {
            score,
            result: ContextSearchResultV1 {
                kind: "file".to_owned(),
                id: format!("workspace:{relative}"),
                label,
                detail: relative.clone(),
                path: Some(relative),
                description: markdown_excerpt(&content, query),
                url: None,
                fingerprint: Some(fingerprint),
                updated_at_ms,
            },
        })
    }

    fn skill_results(&self, query: &str) -> Vec<RankedResult> {
        let mut skills = BTreeMap::<String, (u8, RankedResult)>::new();
        for root in &self.skill_roots {
            if !root.path.is_dir() {
                continue;
            }
            for entry in WalkDir::new(&root.path)
                .max_depth(8)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| entry.depth() == 0 || !hidden_entry(entry.path()))
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file() && entry.file_name() == "SKILL.md")
            {
                let Some(result) = skill_result(root, entry.path(), query) else {
                    continue;
                };
                let key = result.result.label.to_lowercase();
                if skills
                    .get(&key)
                    .is_none_or(|(priority, _)| root.priority < *priority)
                {
                    skills.insert(key, (root.priority, result));
                }
            }
        }
        skills.into_values().map(|(_, result)| result).collect()
    }
}

fn skill_result(root: &SkillRoot, path: &Path, query: &str) -> Option<RankedResult> {
    let relative_dir = path.parent()?.strip_prefix(&root.path).ok()?;
    if !safe_relative_path(relative_dir) {
        return None;
    }
    let body = fs::read_to_string(path).ok()?;
    let frontmatter = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
        .map(|(value, _)| value)
        .and_then(|value| serde_yaml::from_str::<serde_yaml::Value>(value).ok());
    let fallback = relative_dir.file_name()?.to_string_lossy().into_owned();
    let name = frontmatter
        .as_ref()
        .and_then(|value| value.get("name"))
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&fallback)
        .to_owned();
    let description = frontmatter
        .as_ref()
        .and_then(|value| value.get("description"))
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Active Agent skill")
        .chars()
        .take(MAX_EXCERPT_CHARS)
        .collect::<String>();
    let relative = relative_dir.to_string_lossy().replace('\\', "/");
    let score = lexical_score(&name, &relative, &description, query)?;
    let metadata = fs::symlink_metadata(path).ok()?;
    Some(RankedResult {
        score,
        result: ContextSearchResultV1 {
            kind: "skill".to_owned(),
            id: format!("skill:{name}"),
            label: name,
            detail: root.label.to_owned(),
            path: None,
            description: Some(description),
            url: None,
            fingerprint: Some(content_fingerprint(body.as_bytes())),
            updated_at_ms: modified_at_ms(&metadata),
        },
    })
}

#[derive(Debug, Deserialize)]
struct ProjectList {
    #[serde(default)]
    projects: Vec<ProjectListItem>,
}

#[derive(Debug, Deserialize)]
struct ProjectListItem {
    slug: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    outputs: Vec<ProjectOutput>,
}

#[derive(Debug, Deserialize)]
struct ProjectOutput {
    #[serde(default)]
    output_name: String,
    #[serde(default)]
    site_name: String,
    #[serde(default)]
    output_url: String,
    #[serde(default)]
    site_url: String,
    site_id: Option<String>,
}

fn site_results(payload: &str, query: &str) -> Vec<RankedResult> {
    let Ok(projects) = serde_json::from_str::<ProjectList>(payload) else {
        return Vec::new();
    };
    projects
        .projects
        .into_iter()
        .flat_map(|project| {
            project.outputs.into_iter().filter_map(move |output| {
                let label = if output.output_name.is_empty() {
                    output.site_name
                } else {
                    output.output_name
                };
                let url = if output.output_url.is_empty() {
                    output.site_url
                } else {
                    output.output_url
                };
                if label.is_empty() || url.is_empty() {
                    return None;
                }
                let detail = format!("{} · {}", project.slug, url);
                let score = lexical_score(&label, &detail, &project.role, query)?;
                Some(RankedResult {
                    score,
                    result: ContextSearchResultV1 {
                        kind: "site".to_owned(),
                        id: output.site_id.unwrap_or_else(|| url.clone()),
                        label,
                        detail,
                        path: None,
                        description: Some(if project.role.is_empty() {
                            "Finite Site".to_owned()
                        } else {
                            format!("{} access", project.role)
                        }),
                        url: Some(url),
                        fingerprint: None,
                        updated_at_ms: 0,
                    },
                })
            })
        })
        .collect()
}

fn lexical_score(label: &str, path: &str, content: &str, query: &str) -> Option<u16> {
    if query.is_empty() {
        return Some(1);
    }
    let label = label.to_lowercase();
    let path = path.to_lowercase();
    let content = content.to_lowercase();
    Some(if label == query {
        100
    } else if label.starts_with(query) {
        90
    } else if label.contains(query) {
        80
    } else if path.contains(query) {
        65
    } else if content.contains(query) {
        40
    } else {
        return None;
    })
}

fn markdown_excerpt(content: &str, query: &str) -> Option<String> {
    let line = content.lines().map(str::trim).find(|line| {
        !line.is_empty() && (query.is_empty() || line.to_lowercase().contains(query))
    })?;
    Some(
        line.trim_start_matches('#')
            .trim()
            .chars()
            .take(MAX_EXCERPT_CHARS)
            .collect(),
    )
}

fn hidden_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.starts_with('.'))
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(value) if !value.is_empty()))
}

fn modified_at_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn content_fingerprint(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

const fn default_limit() -> u16 {
    30
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{ContextCatalog, ContextSearchRequestV1};

    #[test]
    fn search_returns_workspace_markdown_and_active_skills_only() {
        let root = tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let hermes = root.path().join("hermes");
        fs::create_dir_all(workspace.join("plans")).unwrap();
        fs::create_dir_all(workspace.join(".hidden")).unwrap();
        fs::create_dir_all(hermes.join("skills/strategy-review")).unwrap();
        fs::write(
            workspace.join("plans/pricing-plan.md"),
            "# Pricing plan\nCompare annual and monthly pricing.",
        )
        .unwrap();
        fs::write(workspace.join("plans/secret.txt"), "pricing").unwrap();
        fs::write(workspace.join(".hidden/private.md"), "pricing").unwrap();
        fs::write(
            hermes.join("skills/strategy-review/SKILL.md"),
            "---\nname: strategy-review\ndescription: Review a product strategy.\n---\n",
        )
        .unwrap();

        let catalog = ContextCatalog::new(workspace, hermes, Vec::<PathBuf>::new());
        let pricing = catalog
            .search(
                ContextSearchRequestV1 {
                    query: "pricing".to_owned(),
                    limit: 30,
                },
                None,
            )
            .unwrap();
        assert_eq!(pricing.len(), 1);
        assert_eq!(pricing[0].path.as_deref(), Some("plans/pricing-plan.md"));

        let strategy = catalog
            .search(
                ContextSearchRequestV1 {
                    query: "strategy".to_owned(),
                    limit: 30,
                },
                None,
            )
            .unwrap();
        assert_eq!(strategy.len(), 1);
        assert_eq!(strategy[0].kind, "skill");
        assert_eq!(strategy[0].label, "strategy-review");
    }

    #[test]
    fn search_includes_editable_finite_sites_from_fsite_output() {
        let root = tempdir().unwrap();
        let catalog = ContextCatalog::new(
            root.path().join("workspace"),
            root.path().join("hermes"),
            Vec::<PathBuf>::new(),
        );
        let results = catalog
            .search(
                ContextSearchRequestV1 {
                    query: "store".to_owned(),
                    limit: 30,
                },
                Some(
                    r#"{"projects":[{"slug":"commerce","role":"owner","outputs":[{"output_name":"finite-store","output_url":"https://finite-store.finite.chat/","site_id":"site_1"}]}]}"#,
                ),
            )
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, "site");
        assert_eq!(
            results[0].url.as_deref(),
            Some("https://finite-store.finite.chat/")
        );
    }
}
