//! Project Config (`finite.toml`) parsing and validation.
//!
//! This module is shared by the CLI and server because `finite.toml` is the
//! contract agents read, write, commit, and push. The accepted schema is
//! intentionally narrower than TOML itself; unknown keys fail closed so agents
//! learn from deterministic errors instead of server inference.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::limits::{
    MAX_PROJECT_BRANCH_BYTES, MAX_PROJECT_OUTPUT_ID_BYTES, MAX_PROJECT_OUTPUT_PATH_BYTES,
    MAX_PROJECT_SLUG_BYTES,
};
use crate::{ProtoError, names};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub project: ProjectSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<ProjectSiteConfig>,
    /// Deprecated input-only compatibility for pre-static-only finite.toml
    /// files. `parse_project_config_toml` canonicalizes one legacy static
    /// output into `[site]`; v2 responses must serialize `[site]` only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, ProjectOutputConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSiteConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub branch: String,
    pub path: String,
    #[serde(default)]
    pub spa: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedProjectSiteConfig {
    pub name: String,
    pub branch: String,
    pub path: String,
    pub spa: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOutputConfig {
    pub kind: ProjectOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_name: Option<String>,
    pub branch: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default)]
    pub spa: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectOutputKind {
    Site,
    Document,
    App,
}

impl ProjectOutputKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectOutputKind::Site => "site",
            ProjectOutputKind::Document => "document",
            ProjectOutputKind::App => "app",
        }
    }
}

impl ProjectOutputConfig {
    pub fn routing_name(&self) -> Result<&str, ProtoError> {
        match self.kind {
            ProjectOutputKind::Site => {
                self.site_name
                    .as_deref()
                    .ok_or(ProtoError::InvalidProjectConfig(
                        "site output needs site_name",
                    ))
            }
            ProjectOutputKind::Document => {
                self.document_name
                    .as_deref()
                    .ok_or(ProtoError::InvalidProjectConfig(
                        "document output needs document_name",
                    ))
            }
            ProjectOutputKind::App => {
                self.site_name
                    .as_deref()
                    .ok_or(ProtoError::InvalidProjectConfig(
                        "app output needs site_name",
                    ))
            }
        }
    }

    pub fn normalized_entry(&self) -> Option<&str> {
        match self.kind {
            ProjectOutputKind::Site | ProjectOutputKind::App => None,
            ProjectOutputKind::Document => self.entry.as_deref(),
        }
    }

    pub fn normalized_start(&self) -> Option<&str> {
        match self.kind {
            ProjectOutputKind::App => self.start.as_deref(),
            ProjectOutputKind::Site | ProjectOutputKind::Document => None,
        }
    }
}

impl ProjectConfig {
    pub fn validate(&self) -> Result<(), ProtoError> {
        validate_project_slug(&self.project.slug)?;
        let _ = self.normalized_site()?;
        Ok(())
    }

    pub fn to_toml_string(&self) -> Result<String, ProtoError> {
        self.validate()?;
        let normalized_site = self.normalized_site()?;
        let site = normalized_site.map(|site| ProjectSiteConfig {
            name: if site.name == self.project.slug {
                None
            } else {
                Some(site.name)
            },
            branch: site.branch,
            path: site.path,
            spa: site.spa,
        });
        let canonical = CanonicalProjectConfig {
            project: self.project.clone(),
            site,
        };
        toml::to_string_pretty(&canonical)
            .map_err(|_| ProtoError::InvalidProjectConfig("cannot encode toml"))
    }

    pub fn normalized_site(&self) -> Result<Option<NormalizedProjectSiteConfig>, ProtoError> {
        if self.site.is_some() && !self.outputs.is_empty() {
            return Err(ProtoError::InvalidProjectConfig(
                "project config cannot set both [site] and [outputs.*]",
            ));
        }
        if let Some(site) = &self.site {
            return normalize_site(&self.project.slug, site);
        }
        legacy_static_site_from_outputs(&self.project.slug, &self.outputs)
    }
}

pub fn parse_project_config_toml(input: &str) -> Result<ProjectConfig, ProtoError> {
    let mut config: ProjectConfig = toml::from_str(input)
        .map_err(|_| ProtoError::InvalidProjectConfig("toml does not match schema"))?;
    config.validate()?;
    if !config.outputs.is_empty() {
        let site = config
            .normalized_site()?
            .expect("nonempty legacy outputs produce one site");
        config.site = Some(ProjectSiteConfig {
            name: Some(site.name),
            branch: site.branch,
            path: site.path,
            spa: site.spa,
        });
        config.outputs.clear();
    }
    Ok(config)
}

#[derive(Serialize)]
struct CanonicalProjectConfig {
    project: ProjectSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    site: Option<ProjectSiteConfig>,
}

fn normalize_site(
    project_slug: &str,
    site: &ProjectSiteConfig,
) -> Result<Option<NormalizedProjectSiteConfig>, ProtoError> {
    let name = site
        .name
        .clone()
        .unwrap_or_else(|| project_slug.to_string());
    names::validate_site_name(&name)?;
    validate_branch_name(&site.branch)?;
    validate_site_path(&site.path)?;
    Ok(Some(NormalizedProjectSiteConfig {
        name,
        branch: site.branch.clone(),
        path: site.path.clone(),
        spa: site.spa,
    }))
}

fn legacy_static_site_from_outputs(
    project_slug: &str,
    outputs: &BTreeMap<String, ProjectOutputConfig>,
) -> Result<Option<NormalizedProjectSiteConfig>, ProtoError> {
    if outputs.is_empty() {
        return Ok(None);
    }
    if outputs.len() > 1 {
        return Err(ProtoError::InvalidProjectConfig(
            "static-only Sites supports at most one Project Site",
        ));
    }
    let (output_id, output) = outputs
        .iter()
        .next()
        .expect("nonempty BTreeMap has one first item");
    validate_output_id(output_id)?;
    match output.kind {
        ProjectOutputKind::Site => {}
        ProjectOutputKind::Document => {
            return Err(ProtoError::InvalidProjectConfig(
                "static-only Sites does not support document outputs",
            ));
        }
        ProjectOutputKind::App => {
            return Err(ProtoError::InvalidProjectConfig(
                "static-only Sites does not support app outputs",
            ));
        }
    }
    if output.document_name.is_some() {
        return Err(ProtoError::InvalidProjectConfig(
            "legacy site output must not set document_name",
        ));
    }
    if output.entry.is_some() {
        return Err(ProtoError::InvalidProjectConfig(
            "legacy site output must not set entry",
        ));
    }
    if output.start.is_some() {
        return Err(ProtoError::InvalidProjectConfig(
            "legacy site output must not set start",
        ));
    }
    let name = output
        .site_name
        .clone()
        .unwrap_or_else(|| project_slug.to_string());
    names::validate_site_name(&name)?;
    validate_branch_name(&output.branch)?;
    validate_site_path(&output.path)?;
    Ok(Some(NormalizedProjectSiteConfig {
        name,
        branch: output.branch.clone(),
        path: output.path.clone(),
        spa: output.spa,
    }))
}

pub fn validate_project_slug(slug: &str) -> Result<(), ProtoError> {
    if slug.len() > MAX_PROJECT_SLUG_BYTES as usize {
        return Err(ProtoError::InvalidProjectConfig("project slug is too long"));
    }
    names::validate_site_name(slug).map_err(|_| {
        ProtoError::InvalidProjectConfig(
            "project slug must be a lowercase DNS label and not reserved",
        )
    })
}

pub fn validate_output_id(output_id: &str) -> Result<(), ProtoError> {
    if output_id.is_empty() {
        return Err(ProtoError::InvalidProjectConfig("output id is empty"));
    }
    if output_id.len() > MAX_PROJECT_OUTPUT_ID_BYTES as usize {
        return Err(ProtoError::InvalidProjectConfig("output id is too long"));
    }
    let bytes = output_id.as_bytes();
    let starts_valid = bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit();
    if !starts_valid {
        return Err(ProtoError::InvalidProjectConfig(
            "output id must start with lowercase letter or digit",
        ));
    }
    let all_valid = bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_');
    if !all_valid {
        return Err(ProtoError::InvalidProjectConfig(
            "output id may contain lowercase letters, digits, hyphen, and underscore",
        ));
    }
    Ok(())
}

fn validate_branch_name(branch: &str) -> Result<(), ProtoError> {
    if branch.is_empty() {
        return Err(ProtoError::InvalidProjectConfig("branch is empty"));
    }
    if branch.len() > MAX_PROJECT_BRANCH_BYTES as usize {
        return Err(ProtoError::InvalidProjectConfig("branch name is too long"));
    }
    if branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.ends_with(".lock")
        || branch.contains("..")
        || branch.contains("//")
    {
        return Err(ProtoError::InvalidProjectConfig(
            "branch name is not a safe deploy branch",
        ));
    }
    let all_valid = branch
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'));
    if !all_valid {
        return Err(ProtoError::InvalidProjectConfig(
            "branch name contains unsupported characters",
        ));
    }
    Ok(())
}

fn validate_site_path(path: &str) -> Result<(), ProtoError> {
    if path.is_empty() {
        return Err(ProtoError::InvalidProjectConfig("site path is empty"));
    }
    if path.len() > MAX_PROJECT_OUTPUT_PATH_BYTES as usize {
        return Err(ProtoError::InvalidProjectConfig("site path is too long"));
    }
    if path == "." {
        return Ok(());
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains('\\') {
        return Err(ProtoError::InvalidProjectConfig(
            "site path must be relative",
        ));
    }
    // Bounded by MAX_PROJECT_OUTPUT_PATH_BYTES.
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(ProtoError::InvalidProjectConfig(
                "site path contains an invalid component",
            ));
        }
        if matches!(component, ".git" | ".finite" | "node_modules") {
            return Err(ProtoError::InvalidProjectConfig(
                "site path targets a forbidden directory",
            ));
        }
        let all_safe = component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !all_safe {
            return Err(ProtoError::InvalidProjectConfig(
                "site path contains unsupported characters",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> ProjectConfig {
        ProjectConfig {
            project: ProjectSection {
                slug: "finitechat-native".to_string(),
            },
            site: Some(ProjectSiteConfig {
                name: Some("finitechat-native-mockup".to_string()),
                branch: "main".to_string(),
                path: ".".to_string(),
                spa: false,
            }),
            outputs: BTreeMap::new(),
        }
    }

    #[test]
    fn parses_and_round_trips_static_site_schema() {
        let raw = r#"
[project]
slug = "finitechat-native"

[site]
name = "finitechat-native-mockup"
branch = "main"
path = "."
spa = false
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        assert_eq!(parsed, valid_config());
        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("[project]"));
        assert!(encoded.contains("[site]"));
        assert!(!encoded.contains("[outputs."));
    }

    #[test]
    fn site_name_defaults_to_project_slug() {
        let raw = r#"
[project]
slug = "finitechat-native"

[site]
branch = "main"
path = "."
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        assert_eq!(
            parsed.normalized_site().unwrap().unwrap().name,
            "finitechat-native"
        );
    }

    #[test]
    fn project_only_config_is_a_bare_project_repository() {
        let raw = r#"
[project]
slug = "finite-skills"
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        assert_eq!(parsed.project.slug, "finite-skills");
        assert!(parsed.site.is_none());
        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("[project]"));
        assert!(!encoded.contains("[site]"));
        assert!(!encoded.contains("[outputs."));
    }

    #[test]
    fn parses_legacy_static_output_as_deprecated_input() {
        let raw = r#"
[project]
slug = "finitechat-native"

[outputs.mockup]
kind = "site"
site_name = "finitechat-native-mockup"
branch = "main"
path = "."
spa = false
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        assert_eq!(parsed, valid_config());
        assert!(parsed.outputs.is_empty());
        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("[site]"));
        assert!(!encoded.contains("[outputs.mockup]"));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        let unknown = r#"
[project]
slug = "finitechat-native"
extra = "nope"

[site]
name = "finitechat-native-mockup"
branch = "main"
path = "."
"#;
        assert!(matches!(
            parse_project_config_toml(unknown),
            Err(ProtoError::InvalidProjectConfig(_))
        ));

        let mut config = valid_config();
        config.site.as_mut().unwrap().branch = "../main".to_string();
        assert_eq!(
            config.validate(),
            Err(ProtoError::InvalidProjectConfig(
                "branch name is not a safe deploy branch"
            ))
        );

        let mut config = valid_config();
        config.site.as_mut().unwrap().path = "node_modules".to_string();
        assert_eq!(
            config.validate(),
            Err(ProtoError::InvalidProjectConfig(
                "site path targets a forbidden directory"
            ))
        );

        let raw = r#"
[project]
slug = "tiny-crm"

[outputs.web]
kind = "app"
site_name = "tiny-crm"
branch = "main"
path = "app"
"#;
        assert_eq!(
            parse_project_config_toml(raw),
            Err(ProtoError::InvalidProjectConfig(
                "static-only Sites does not support app outputs"
            ))
        );

        let raw = r#"
[project]
slug = "tiny-crm"

[outputs.web]
kind = "app"
site_name = "tiny-crm"
branch = "main"
path = "app"
start = "python app.py"
"#;
        assert_eq!(
            parse_project_config_toml(raw),
            Err(ProtoError::InvalidProjectConfig(
                "static-only Sites does not support app outputs"
            ))
        );

        let raw = r#"
[project]
slug = "hermes-notes"

[outputs.doc]
kind = "document"
document_name = "hermes"
branch = "main"
path = "docs"
entry = "start.html"
"#;
        assert_eq!(
            parse_project_config_toml(raw),
            Err(ProtoError::InvalidProjectConfig(
                "static-only Sites does not support document outputs"
            ))
        );

        let raw = r#"
[project]
slug = "multi"

[outputs.one]
kind = "site"
site_name = "multi-one"
branch = "main"
path = "one"

[outputs.two]
kind = "site"
site_name = "multi-two"
branch = "main"
path = "two"
"#;
        assert_eq!(
            parse_project_config_toml(raw),
            Err(ProtoError::InvalidProjectConfig(
                "static-only Sites supports at most one Project Site"
            ))
        );
    }
}
