//! Capture Git's portable object graph, never a live copy of mutable pack/ref
//! files. Bundle creation happens in a private repository with fixed refs.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{BackupError, Entry, add_entry, fs, limits, private_dir};

#[path = "process.rs"]
mod process;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectArchive {
    pub(super) project_id: String,
    head: String,
    refs: BTreeMap<String, String>,
    retained: Vec<String>,
    bundle: Option<String>,
}

#[derive(PartialEq, Eq)]
pub(super) struct Observed {
    head: String,
    refs: BTreeMap<String, String>,
}

impl ProjectArchive {
    pub(super) fn items(&self) -> usize {
        1 + self.refs.len() + self.retained.len()
    }

    pub(super) fn tree_entries(&self) -> Result<u64, BackupError> {
        let mut count = u64::from(limits::BACKUP_GIT_FIXED_TREE_ENTRIES);
        for name in self.refs.keys() {
            count += ref_depth(name)?;
        }
        // Each reserved ref has six path components under the restore root.
        Ok(count + self.retained.len() as u64 * 6)
    }

    pub(super) fn matches(&self, observed: &Option<Observed>) -> bool {
        match observed {
            Some(observed) => self.head == observed.head && self.refs == observed.refs,
            None => false,
        }
    }
}

impl Observed {
    pub(super) fn size(&self) -> (usize, u64) {
        let bytes = self.refs.iter().fold(self.head.len(), |sum, (name, sha)| {
            sum + name.len() + sha.len()
        });
        (self.refs.len(), bytes as u64)
    }
}

pub(super) fn observe(data: &Path, project_id: &str) -> Result<Option<Observed>, BackupError> {
    validate_id(project_id)?;
    let source = data.join("git/projects").join(format!("{project_id}.git"));
    if !source.try_exists()? {
        return Ok(None);
    }
    Ok(Some(Observed {
        head: command(&source, &["symbolic-ref", "HEAD"])?,
        refs: refs(&source)?,
    }))
}

pub(super) fn capture(
    data: &Path,
    project_id: &str,
    events: &[finitesites_store::GitRefEventRecord],
    staging: &Path,
    repository: &Path,
    entries: &mut Vec<Entry>,
    count: &mut u32,
) -> Result<ProjectArchive, BackupError> {
    validate_id(project_id)?;
    let mut retained = std::collections::BTreeSet::new();
    for event in events.iter().filter(|event| event.project_id == project_id) {
        for sha in [&event.old_sha, &event.new_sha] {
            if sha != &"0".repeat(40) {
                if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(BackupError::Invalid("invalid historical Git object"));
                }
                retained.insert(sha.clone());
            }
        }
    }
    let retained = retained.into_iter().collect::<Vec<_>>();
    let source = data.join("git/projects").join(format!("{project_id}.git"));
    if !source.try_exists()? {
        return Err(BackupError::Invalid("missing project repository"));
    }
    let refs = refs(&source)?;
    validate_recorded_refs(project_id, events, &refs)?;
    let head = command(&source, &["symbolic-ref", "HEAD"])?;
    let fixed = staging.join(format!("{project_id}.git"));
    private_dir(&fixed)?;
    command(&fixed, &["init", "--bare", "--initial-branch=main"])?;
    let objects = source.join("objects").canonicalize()?;
    let objects = objects
        .to_str()
        .ok_or(BackupError::Invalid("non-UTF8 Git object path"))?;
    if objects.contains(['\r', '\n']) {
        return Err(BackupError::Invalid("invalid Git object path"));
    }
    fs::write(
        fixed.join("objects/info/alternates"),
        format!("{objects}\n"),
    )?;
    for (name, sha) in &refs {
        command(&fixed, &["update-ref", name, sha])?;
    }
    for sha in &retained {
        if refs
            .get(&format!("refs/finite-recovery/{sha}"))
            .is_some_and(|existing| existing != sha)
        {
            return Err(BackupError::Invalid(
                "Git recovery ref conflicts with source ref",
            ));
        }
        command(
            &fixed,
            &["update-ref", &format!("refs/finite-recovery/{sha}"), sha],
        )?;
    }
    let bundle = if refs.is_empty() && retained.is_empty() {
        None
    } else {
        let path = format!("git-bundles/{project_id}.bundle");
        private_dir(&staging.join("git-bundles"))?;
        let output = staging.join(&path);
        // Git writes to a pipe so the byte limit is enforced before disk writes.
        let mut file = fs::File::create_new(&output)?;
        process::run(
            process::git(&fixed, &["bundle", "create", "-", "--all"])?,
            &mut file,
            limits::MAX_BACKUP_OBJECT_BYTES,
        )?;
        file.sync_all()?;
        add_entry(repository, &output, &path, entries, count)?;
        Some(path)
    };
    if refs != self::refs(&source)? || head != command(&source, &["symbolic-ref", "HEAD"])? {
        return Err(BackupError::Invalid(
            "Git refs changed during capture; retry",
        ));
    }
    Ok(ProjectArchive {
        project_id: project_id.into(),
        head,
        refs,
        retained,
        bundle,
    })
}

pub(super) fn restore(
    staging: &Path,
    project: &ProjectArchive,
    events: &[finitesites_store::GitRefEventRecord],
) -> Result<(), BackupError> {
    validate_id(&project.project_id)?;
    let required: std::collections::BTreeSet<_> = events
        .iter()
        .filter(|event| event.project_id == project.project_id)
        .flat_map(|event| [&event.old_sha, &event.new_sha])
        .filter(|sha| sha.as_str() != "0".repeat(40))
        .cloned()
        .collect();
    if project
        .retained
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        != required
    {
        return Err(BackupError::Invalid("manifest omits recorded Git history"));
    }
    validate_recorded_refs(&project.project_id, events, &project.refs)?;
    let target = staging
        .join("git/projects")
        .join(format!("{}.git", project.project_id));
    private_dir(&target)?;
    command(&target, &["init", "--bare", "--initial-branch=main"])?;
    match &project.bundle {
        Some(bundle) if bundle == &format!("git-bundles/{}.bundle", project.project_id) => {
            let bundle = staging.join(bundle);
            let path = bundle
                .to_str()
                .ok_or(BackupError::Invalid("non-UTF8 bundle path"))?;
            command(&target, &["bundle", "verify", path])?;
            command(
                &target,
                &[
                    "-c",
                    "fetch.unpackLimit=1",
                    "-c",
                    "core.logAllRefUpdates=false",
                    "fetch",
                    "--quiet",
                    path,
                    "+refs/*:refs/*",
                ],
            )?;
        }
        None if project.refs.is_empty() && project.retained.is_empty() => {}
        _ => return Err(BackupError::Invalid("project bundle is missing or invalid")),
    }
    command(&target, &["check-ref-format", &project.head])?;
    command(&target, &["symbolic-ref", "HEAD", &project.head])?;
    let mut expected = project.refs.clone();
    for sha in &project.retained {
        expected.insert(format!("refs/finite-recovery/{sha}"), sha.clone());
    }
    if refs(&target)? != expected {
        return Err(BackupError::Invalid("restored Git refs differ"));
    }
    command(&target, &["fsck", "--full"])?;
    Ok(())
}

// The legacy event log deduplicates transitions and has no authoritative ref
// cursor. Only an unambiguous chain can establish its terminal state; never
// infer ordering from timestamps or row IDs. Complex histories fail closed.
fn validate_recorded_refs(
    project_id: &str,
    events: &[finitesites_store::GitRefEventRecord],
    observed: &BTreeMap<String, String>,
) -> Result<(), BackupError> {
    let mut by_ref: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    for event in events.iter().filter(|event| event.project_id == project_id) {
        if event.ref_name.starts_with("refs/finite-recovery/") {
            return Err(BackupError::Invalid("recorded reserved recovery ref"));
        }
        if by_ref
            .entry(&event.ref_name)
            .or_default()
            .insert(&event.old_sha, &event.new_sha)
            .is_some()
        {
            return Err(BackupError::Invalid(
                "ambiguous Git history; cannot checkpoint",
            ));
        }
    }
    let mut expected = BTreeMap::new();
    for (name, edges) in by_ref {
        let targets: std::collections::BTreeSet<_> = edges.values().copied().collect();
        let roots: Vec<_> = edges
            .keys()
            .filter(|sha| !targets.contains(**sha))
            .copied()
            .collect();
        if roots.len() != 1 || targets.len() != edges.len() {
            return Err(BackupError::Invalid(
                "ambiguous Git history; cannot checkpoint",
            ));
        }
        let mut cursor = roots[0];
        let mut traversed = 0;
        while let Some(next) = edges.get(cursor) {
            traversed += 1;
            if traversed > edges.len() {
                return Err(BackupError::Invalid(
                    "cyclic Git history; cannot checkpoint",
                ));
            }
            cursor = next;
        }
        if traversed != edges.len() {
            return Err(BackupError::Invalid(
                "disconnected Git history; cannot checkpoint",
            ));
        }
        if cursor != "0".repeat(40) {
            expected.insert(name.to_string(), cursor.to_string());
        }
    }
    let mut actual = BTreeMap::new();
    for (name, sha) in observed {
        if let Some(retained) = name.strip_prefix("refs/finite-recovery/") {
            if retained != sha {
                return Err(BackupError::Invalid("invalid recovery ref"));
            }
        } else {
            actual.insert(name.clone(), sha.clone());
        }
    }
    if actual != expected {
        return Err(BackupError::Invalid(
            "Git refs differ from recorded transitions",
        ));
    }
    Ok(())
}

fn refs(repo: &Path) -> Result<BTreeMap<String, String>, BackupError> {
    let output = command(repo, &["for-each-ref", "--format=%(objectname) %(refname)"])?;
    let mut refs = BTreeMap::new();
    for line in output.lines().take(limits::MAX_BACKUP_OBJECTS as usize + 1) {
        let (sha, name) = line
            .split_once(' ')
            .ok_or(BackupError::Invalid("invalid Git ref listing"))?;
        if sha.len() != 40
            || !sha.bytes().all(|c| c.is_ascii_hexdigit())
            || !name.starts_with("refs/")
        {
            return Err(BackupError::Invalid("invalid Git ref"));
        }
        ref_depth(name)?;
        refs.insert(name.into(), sha.into());
    }
    if refs.len() > limits::MAX_BACKUP_OBJECTS as usize {
        return Err(BackupError::Invalid("too many Git refs"));
    }
    Ok(refs)
}

fn ref_depth(name: &str) -> Result<u64, BackupError> {
    let depth = 3 + Path::new(name).components().count() as u64;
    if depth > u64::from(limits::MAX_BACKUP_RESTORE_TREE_DEPTH) {
        return Err(BackupError::Invalid("Git ref exceeds restore depth"));
    }
    Ok(depth)
}

fn validate_id(id: &str) -> Result<(), BackupError> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(BackupError::Invalid("invalid project id"));
    }
    Ok(())
}

fn command(repo: &Path, args: &[&str]) -> Result<String, BackupError> {
    let mut output = Vec::new();
    process::run(
        process::git(repo, args)?,
        &mut output,
        limits::MAX_BACKUP_MANIFEST_BYTES,
    )?;
    String::from_utf8(output)
        .map(|s| s.trim().into())
        .map_err(|_| BackupError::Invalid("non-UTF8 Git output"))
}
