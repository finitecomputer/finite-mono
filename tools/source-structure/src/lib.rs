//! Small, syntax-aware source limits. Scope grows as components are cleaned up.

use anyhow::{Context, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rust_roots: Vec<String>,
    pub max_rust_lines: usize,
    pub max_struct_fields: usize,
    pub guides: BTreeMap<String, usize>,
    pub struct_exceptions: BTreeMap<String, StructException>,
}

/// Exact counts make exceptions a ratchet: growth fails, shrinkage retires them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructException {
    pub fields: usize,
    pub reason: String,
}

pub fn check_repository(root: &Path, config: &Config) -> anyhow::Result<Vec<String>> {
    ensure!(
        !config.rust_roots.is_empty(),
        "rust_roots must not be empty"
    );
    ensure!(config.max_rust_lines > 0, "max_rust_lines must be positive");
    ensure!(
        config.max_struct_fields > 0,
        "max_struct_fields must be positive"
    );
    for (key, exception) in &config.struct_exceptions {
        ensure!(
            !exception.reason.trim().is_empty(),
            "{key}: exception needs a reason"
        );
        ensure!(
            exception.fields > config.max_struct_fields,
            "{key}: exception does not exceed the default field limit; remove it"
        );
    }

    let mut files = BTreeSet::new();
    for relative in &config.rust_roots {
        validate_relative_path(relative)?;
        let mut scoped_files = BTreeSet::new();
        collect_rust_files(&root.join(relative), &mut scoped_files)?;
        ensure!(
            !scoped_files.is_empty(),
            "{relative}: no Rust source files found"
        );
        files.extend(scoped_files);
    }

    let mut diagnostics = Vec::new();
    let mut seen_exceptions = BTreeSet::new();
    for file in files {
        let relative = file
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let source =
            std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        check_lines(&relative, &source, config.max_rust_lines, &mut diagnostics);
        let parsed = match syn::parse_file(&source) {
            Ok(parsed) => parsed,
            Err(error) => {
                diagnostics.push(format!(
                    "{relative}:{}: cannot parse Rust: {error}",
                    error.span().start().line
                ));
                continue;
            }
        };
        let mut visitor = StructureVisitor {
            path: &relative,
            config,
            diagnostics: &mut diagnostics,
            seen_exceptions: &mut seen_exceptions,
            modules: Vec::new(),
        };
        visitor.visit_file(&parsed);
    }
    for key in config.struct_exceptions.keys() {
        if !seen_exceptions.contains(key) {
            diagnostics.push(format!(
                "{key}: stale struct exception; remove it or update its path"
            ));
        }
    }
    for (relative, limit) in &config.guides {
        validate_relative_path(relative)?;
        ensure!(*limit > 0, "{relative}: guide limit must be positive");
        let source = std::fs::read_to_string(root.join(relative))
            .with_context(|| format!("read guide {relative}"))?;
        check_lines(relative, &source, *limit, &mut diagnostics);
    }
    diagnostics.sort();
    Ok(diagnostics)
}

fn validate_relative_path(path: &str) -> anyhow::Result<()> {
    ensure!(
        !path.is_empty()
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "scope paths must be repository-relative without '.' or '..': {path}"
    );
    Ok(())
}

fn collect_rust_files(path: &Path, files: &mut BTreeSet<PathBuf>) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("read scope {}", path.display()))?;
    ensure!(
        !metadata.is_symlink(),
        "symlink in source scope: {}",
        path.display()
    );
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect_rust_files(&entry?.path(), files)?;
        }
    } else if path.extension().is_some_and(|extension| extension == "rs") {
        files.insert(path.to_owned());
    }
    Ok(())
}

fn check_lines(path: &str, source: &str, limit: usize, diagnostics: &mut Vec<String>) {
    let lines = source.lines().count();
    if lines > limit {
        diagnostics.push(format!(
            "{path}:1: {lines} lines exceeds {limit}; split by responsibility"
        ));
    }
}

struct StructureVisitor<'a> {
    path: &'a str,
    config: &'a Config,
    diagnostics: &'a mut Vec<String>,
    seen_exceptions: &'a mut BTreeSet<String>,
    modules: Vec<String>,
}

impl StructureVisitor<'_> {
    fn report(&mut self, line: usize, message: impl std::fmt::Display) {
        self.diagnostics
            .push(format!("{}:{line}: {message}", self.path));
    }
}

impl<'ast> Visit<'ast> for StructureVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if node.content.is_some() {
            self.report(
                node.span().start().line,
                format!(
                    "inline module {}; use a separate file (including tests)",
                    node.ident
                ),
            );
        }
        self.modules.push(node.ident.to_string());
        visit::visit_item_mod(self, node);
        self.modules.pop();
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        let mut names = self.modules.clone();
        names.push(node.ident.to_string());
        let key = format!("{}::{}", self.path, names.join("::"));
        let fields = node.fields.len();
        if let Some(exception) = self.config.struct_exceptions.get(&key) {
            self.seen_exceptions.insert(key.clone());
            if fields != exception.fields {
                self.report(node.span().start().line, format!("{key} has {fields} fields, exception requires exactly {}; review or remove the exception", exception.fields));
            }
        } else if fields > self.config.max_struct_fields {
            self.report(node.span().start().line, format!("{key} has {fields} fields, limit is {}; group internal state by responsibility, or document an exact contract exception", self.config.max_struct_fields));
        }
        visit::visit_item_struct(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if node
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "include")
        {
            self.report(
                node.span().start().line,
                "include! hides source structure; use an external module",
            );
        }
        visit::visit_macro(self, node);
    }
}
