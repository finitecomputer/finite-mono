//! Read-only capture: never initialize or migrate the source registry.

use std::path::Path;
use std::time::{Duration, Instant};

use finitesites_proto::{hex, limits};
use rusqlite::{Connection, OpenFlags, backup::StepResult};

use crate::{GitRefEventRecord, GitRefEventStatus, Store, StoreError};

#[derive(Debug, PartialEq, Eq)]
pub struct Inventory {
    pub blobs: Vec<(String, u64)>,
    pub projects: Vec<String>,
    pub git_events: Vec<GitRefEventRecord>,
}

pub fn capture(source: &Path, destination: &Path) -> Result<Inventory, StoreError> {
    let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // The caller reserves a private scratch file, never an existing database.
    let mut destination =
        Connection::open_with_flags(destination, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let started = Instant::now();
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
        loop {
            if started.elapsed() > Duration::from_secs(limits::MAX_BACKUP_CAPTURE_SECONDS) {
                return Err(StoreError::Conflict("registry capture timed out"));
            }
            let result = backup.step(256)?;
            let pages = backup.progress().pagecount;
            let page_size: u64 = source.pragma_query_value(None, "page_size", |row| row.get(0))?;
            if (pages.max(0) as u64).saturating_mul(page_size) > limits::MAX_BACKUP_OBJECT_BYTES {
                return Err(StoreError::Conflict("registry exceeds backup size limit"));
            }
            if result == StepResult::Done {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    inventory(&destination)
}

pub fn inspect(scratch: &Path) -> Result<Inventory, StoreError> {
    inventory(&Connection::open_with_flags(
        scratch,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

fn inventory(conn: &Connection) -> Result<Inventory, StoreError> {
    let check: String = conn.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if check != "ok" || conn.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(StoreError::CorruptState(
            "recovery registry integrity failed",
        ));
    }
    if conn.prepare("SELECT 1 FROM version_files f LEFT JOIN blobs b ON b.sha256 = f.sha256 WHERE b.sha256 IS NULL OR b.size != f.size")?.exists([])? {
        return Err(StoreError::CorruptState("version references missing or inconsistent blob"));
    }
    let mut statement = conn.prepare("SELECT sha256, size FROM blobs ORDER BY sha256 LIMIT ?1")?;
    let rows = statement.query_map([limits::MAX_BACKUP_OBJECTS + 1], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })?;
    let mut blobs = Vec::new();
    // SQL bounds the source independently of the live registry's size.
    for row in rows {
        let (hash, size) = row?;
        if !hex::is_hex32(&hash) || size > limits::MAX_FILE_BYTES {
            return Err(StoreError::CorruptState("invalid recovery blob"));
        }
        blobs.push((hash, size));
    }
    let mut statement = conn.prepare("SELECT id FROM projects ORDER BY id LIMIT ?1")?;
    let projects = statement
        .query_map([limits::MAX_BACKUP_OBJECTS + 1], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    if blobs.len() + projects.len() > limits::MAX_BACKUP_OBJECTS as usize {
        return Err(StoreError::Conflict("recovery inventory exceeds limit"));
    }
    let mut statement = conn.prepare("SELECT id, project_id, ref_name, old_sha, new_sha, actor_principal_id, actor_agent_key_id, git_credential_id, project_output_id, status, version_id, error FROM git_ref_events ORDER BY id LIMIT ?1")?;
    let mut git_events = Vec::new();
    for event in statement.query_map(
        [limits::MAX_BACKUP_OBJECTS + 1],
        Store::row_to_git_ref_event,
    )? {
        git_events.push(event??);
    }
    if git_events.len() > limits::MAX_BACKUP_OBJECTS as usize {
        return Err(StoreError::Conflict("Git recovery inventory exceeds limit"));
    }
    if git_events
        .iter()
        .any(|event| event.status == GitRefEventStatus::Pending)
    {
        return Err(StoreError::Conflict(
            "Git reconciliation is pending; retry capture",
        ));
    }
    Ok(Inventory {
        blobs,
        projects,
        git_events,
    })
}
