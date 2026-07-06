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
    MAX_PROJECT_OUTPUTS, MAX_PROJECT_SLUG_BYTES, MAX_START_COMMAND_BYTES,
};
use crate::{ProtoError, names};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub project: ProjectSection,
    #[serde(default)]
    pub outputs: BTreeMap<String, ProjectOutputConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    pub slug: String,
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
        if self.outputs.len() > MAX_PROJECT_OUTPUTS as usize {
            return Err(ProtoError::InvalidProjectConfig("too many outputs"));
        }
        // Bounded by MAX_PROJECT_OUTPUTS above.
        for (output_id, output) in &self.outputs {
            validate_output_id(output_id)?;
            let routing_name = output.routing_name()?;
            names::validate_site_name(routing_name)?;
            match output.kind {
                ProjectOutputKind::Site => {
                    if output.document_name.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "site output must not set document_name",
                        ));
                    }
                    if output.entry.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "site output must not set entry",
                        ));
                    }
                    if output.start.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "site output must not set start",
                        ));
                    }
                }
                ProjectOutputKind::Document => {
                    if output.site_name.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "document output must not set site_name",
                        ));
                    }
                    if output.spa {
                        return Err(ProtoError::InvalidProjectConfig(
                            "document output must not set spa",
                        ));
                    }
                    if let Some(entry) = output.entry.as_deref() {
                        validate_document_entry(entry)?;
                    }
                    if output.start.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "document output must not set start",
                        ));
                    }
                }
                ProjectOutputKind::App => {
                    if output.document_name.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "app output must not set document_name",
                        ));
                    }
                    if output.entry.is_some() {
                        return Err(ProtoError::InvalidProjectConfig(
                            "app output must not set entry",
                        ));
                    }
                    if output.spa {
                        return Err(ProtoError::InvalidProjectConfig(
                            "app output must not set spa",
                        ));
                    }
                    let Some(start) = output.start.as_deref() else {
                        return Err(ProtoError::InvalidProjectConfig("app output needs start"));
                    };
                    validate_start_command(start)?;
                }
            }
            validate_branch_name(&output.branch)?;
            validate_output_path(&output.path)?;
        }
        Ok(())
    }

    pub fn to_toml_string(&self) -> Result<String, ProtoError> {
        self.validate()?;
        toml::to_string_pretty(self)
            .map_err(|_| ProtoError::InvalidProjectConfig("cannot encode toml"))
    }
}

pub fn parse_project_config_toml(input: &str) -> Result<ProjectConfig, ProtoError> {
    let config: ProjectConfig = toml::from_str(input)
        .map_err(|_| ProtoError::InvalidProjectConfig("toml does not match schema"))?;
    config.validate()?;
    Ok(config)
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

fn validate_output_path(path: &str) -> Result<(), ProtoError> {
    if path.is_empty() {
        return Err(ProtoError::InvalidProjectConfig("output path is empty"));
    }
    if path.len() > MAX_PROJECT_OUTPUT_PATH_BYTES as usize {
        return Err(ProtoError::InvalidProjectConfig("output path is too long"));
    }
    if path == "." {
        return Ok(());
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains('\\') {
        return Err(ProtoError::InvalidProjectConfig(
            "output path must be relative",
        ));
    }
    // Bounded by MAX_PROJECT_OUTPUT_PATH_BYTES.
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(ProtoError::InvalidProjectConfig(
                "output path contains an invalid component",
            ));
        }
        if matches!(component, ".git" | ".finite" | "node_modules") {
            return Err(ProtoError::InvalidProjectConfig(
                "output path targets a forbidden directory",
            ));
        }
        let all_safe = component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !all_safe {
            return Err(ProtoError::InvalidProjectConfig(
                "output path contains unsupported characters",
            ));
        }
    }
    Ok(())
}

fn validate_document_entry(entry: &str) -> Result<(), ProtoError> {
    if entry.is_empty() {
        return Err(ProtoError::InvalidProjectConfig("document entry is empty"));
    }
    if entry.starts_with('/') || entry.ends_with('/') || entry.contains('\\') {
        return Err(ProtoError::InvalidProjectConfig(
            "document entry must be a relative markdown path",
        ));
    }
    if !entry.ends_with(".md") {
        return Err(ProtoError::InvalidProjectConfig(
            "document entry must end with .md",
        ));
    }
    // Bounded by MAX_PROJECT_OUTPUT_PATH_BYTES because entry lives inside the
    // same Project Output path namespace.
    validate_output_path(entry)
}

fn validate_start_command(start: &str) -> Result<(), ProtoError> {
    if start.is_empty() {
        return Err(ProtoError::InvalidProjectConfig("app start is empty"));
    }
    if start.len() > MAX_START_COMMAND_BYTES as usize {
        return Err(ProtoError::InvalidProjectConfig("app start is too long"));
    }
    if start.trim() != start {
        return Err(ProtoError::InvalidProjectConfig(
            "app start must not have leading or trailing whitespace",
        ));
    }
    if !start.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err(ProtoError::InvalidProjectConfig(
            "app start must be one printable ASCII command line",
        ));
    }
    let first = start.split_whitespace().next().unwrap_or("");
    if !matches!(first, "node" | "bun" | "uv") {
        return Err(ProtoError::InvalidProjectConfig(
            "app start must begin with node, bun, or uv",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> ProjectConfig {
        let mut outputs = BTreeMap::new();
        outputs.insert(
            "mockup".to_string(),
            ProjectOutputConfig {
                kind: ProjectOutputKind::Site,
                site_name: Some("finitechat-native-mockup".to_string()),
                document_name: None,
                branch: "main".to_string(),
                path: ".".to_string(),
                entry: None,
                spa: false,
                start: None,
            },
        );
        ProjectConfig {
            project: ProjectSection {
                slug: "finitechat-native".to_string(),
            },
            outputs,
        }
    }

    #[test]
    fn parses_and_round_trips_minimal_schema() {
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
        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("[project]"));
        assert!(encoded.contains("[outputs.mockup]"));
    }

    #[test]
    fn project_only_config_is_a_bare_project_repository() {
        let raw = r#"
[project]
slug = "finite-skills"
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        assert_eq!(parsed.project.slug, "finite-skills");
        assert!(parsed.outputs.is_empty());
        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("[project]"));
        assert!(!encoded.contains("[outputs."));
    }

    #[test]
    fn parses_document_output_schema() {
        let raw = r#"
[project]
slug = "hermes-notes"

[outputs.doc]
kind = "document"
document_name = "hermes"
branch = "main"
path = "docs"
entry = "start.md"
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        let output = parsed.outputs.get("doc").unwrap();
        assert_eq!(output.kind, ProjectOutputKind::Document);
        assert_eq!(output.routing_name().unwrap(), "hermes");
        assert_eq!(output.normalized_entry(), Some("start.md"));
        assert!(!output.spa);

        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("kind = \"document\""));
        assert!(encoded.contains("document_name = \"hermes\""));
    }

    #[test]
    fn parses_app_output_schema() {
        let raw = r#"
[project]
slug = "tiny-crm"

[outputs.web]
kind = "app"
site_name = "tiny-crm"
branch = "main"
path = "app"
start = "bun server.ts"
"#;
        let parsed = parse_project_config_toml(raw).unwrap();
        let output = parsed.outputs.get("web").unwrap();
        assert_eq!(output.kind, ProjectOutputKind::App);
        assert_eq!(output.routing_name().unwrap(), "tiny-crm");
        assert_eq!(output.normalized_start(), Some("bun server.ts"));
        assert_eq!(output.normalized_entry(), None);
        assert!(!output.spa);

        let encoded = parsed.to_toml_string().unwrap();
        assert!(encoded.contains("kind = \"app\""));
        assert!(encoded.contains("start = \"bun server.ts\""));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        let unknown = r#"
[project]
slug = "finitechat-native"
extra = "nope"

[outputs.mockup]
kind = "site"
site_name = "finitechat-native-mockup"
branch = "main"
path = "."
"#;
        assert!(matches!(
            parse_project_config_toml(unknown),
            Err(ProtoError::InvalidProjectConfig(_))
        ));

        let mut config = valid_config();
        config.outputs.get_mut("mockup").unwrap().branch = "../main".to_string();
        assert_eq!(
            config.validate(),
            Err(ProtoError::InvalidProjectConfig(
                "branch name is not a safe deploy branch"
            ))
        );

        let mut config = valid_config();
        config.outputs.get_mut("mockup").unwrap().path = "node_modules".to_string();
        assert_eq!(
            config.validate(),
            Err(ProtoError::InvalidProjectConfig(
                "output path targets a forbidden directory"
            ))
        );

        let mut config = valid_config();
        let output = config.outputs.get_mut("mockup").unwrap();
        output.kind = ProjectOutputKind::Document;
        output.document_name = Some("hermes".to_string());
        assert_eq!(
            config.validate(),
            Err(ProtoError::InvalidProjectConfig(
                "document output must not set site_name"
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
            Err(ProtoError::InvalidProjectConfig("app output needs start"))
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
                "app start must begin with node, bun, or uv"
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
                "document entry must end with .md"
            ))
        );
    }
}
