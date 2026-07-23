use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use rustix::fs::{FlockOperation, flock};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::search::SEARCH_INDEX_VERSION;
use crate::{
    CliError, EmbeddingProviderAdapter, EmbeddingProviderInput, EmbeddingProviderResponse,
};

const SEMANTIC_SCHEMA_VERSION: &str = "finitebrain-semantic-index-v1";
const EMBEDDING_REQUEST_CHUNK: usize = 64;
const MAX_SEMANTIC_SECTIONS_PER_FOLDER: usize = 100_000;
const SEMANTIC_SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS semantic_settings (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        schema_version TEXT NOT NULL,
        enabled INTEGER NOT NULL,
        lifecycle TEXT NOT NULL,
        active_generation TEXT,
        last_error_class TEXT
     );
     CREATE TABLE IF NOT EXISTS semantic_generations (
        generation_id TEXT PRIMARY KEY,
        model TEXT NOT NULL,
        model_version TEXT NOT NULL,
        dimensions INTEGER NOT NULL,
        section_format TEXT NOT NULL,
        complete INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS semantic_vectors (
        generation_id TEXT NOT NULL,
        section_key TEXT NOT NULL,
        page_path TEXT NOT NULL,
        section_hash TEXT NOT NULL,
        vector BLOB NOT NULL,
        PRIMARY KEY (generation_id, section_key, page_path),
        FOREIGN KEY (generation_id) REFERENCES semantic_generations(generation_id) ON DELETE CASCADE
     );
     CREATE INDEX IF NOT EXISTS semantic_vectors_generation_path
        ON semantic_vectors(generation_id, page_path);";

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SemanticLifecycle {
    Disabled,
    Building,
    Ready,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticStatus {
    pub(crate) enabled: bool,
    pub(crate) lifecycle: SemanticLifecycle,
    pub(crate) model: Option<String>,
    pub(crate) model_version: Option<String>,
    pub(crate) dimensions: Option<usize>,
    pub(crate) current_sections: usize,
    pub(crate) current_vectors: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticCandidate {
    pub(crate) section_key: String,
    pub(crate) page_path: String,
    pub(crate) page_title: String,
    pub(crate) heading_ancestry: Vec<String>,
    pub(crate) heading: Option<String>,
    pub(crate) excerpt: String,
    pub(crate) disposition: String,
    pub(crate) similarity: f32,
}

#[derive(Debug)]
struct RankedSemanticCandidate {
    section_key: String,
    page_path: String,
    page_title: String,
    heading_ancestry: String,
    heading: Option<String>,
    disposition: String,
    similarity: f32,
}

#[derive(Debug)]
struct SectionForEmbedding {
    key: String,
    page_path: String,
    page_title: String,
    heading_ancestry: Vec<String>,
    heading: Option<String>,
    body: String,
    section_hash: String,
}

#[derive(Debug, Clone)]
struct GenerationContract {
    id: String,
    model: String,
    model_version: String,
    dimensions: usize,
}

#[derive(Debug, Clone)]
struct ReusableVector {
    section_hash: String,
    vector: Vec<u8>,
}

pub(crate) fn ensure_schema(connection: &Connection) -> Result<(), CliError> {
    connection
        .execute_batch(SEMANTIC_SCHEMA_SQL)
        .map_err(index_error)?;
    let schema_is_compatible = table_columns(connection, "semantic_settings")?
        == [
            "singleton",
            "schema_version",
            "enabled",
            "lifecycle",
            "active_generation",
            "last_error_class",
        ]
        && table_columns(connection, "semantic_generations")?
            == [
                "generation_id",
                "model",
                "model_version",
                "dimensions",
                "section_format",
                "complete",
            ]
        && table_columns(connection, "semantic_vectors")?
            == [
                "generation_id",
                "section_key",
                "page_path",
                "section_hash",
                "vector",
            ];
    if !schema_is_compatible {
        connection
            .execute_batch(
                "DROP TABLE IF EXISTS semantic_vectors;
                 DROP TABLE IF EXISTS semantic_generations;
                 DROP TABLE IF EXISTS semantic_settings;",
            )
            .map_err(index_error)?;
        connection
            .execute_batch(SEMANTIC_SCHEMA_SQL)
            .map_err(index_error)?;
    }
    connection
        .execute(
            "INSERT INTO semantic_settings (
                singleton, schema_version, enabled, lifecycle, active_generation, last_error_class
             ) VALUES (1, ?1, 1, 'building', NULL, NULL)
             ON CONFLICT(singleton) DO NOTHING",
            [SEMANTIC_SCHEMA_VERSION],
        )
        .map_err(index_error)?;
    let version: String = connection
        .query_row(
            "SELECT schema_version FROM semantic_settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(index_error)?;
    if version != SEMANTIC_SCHEMA_VERSION {
        connection
            .execute_batch(
                "DELETE FROM semantic_vectors;
                 DELETE FROM semantic_generations;
                 UPDATE semantic_settings
                    SET schema_version = 'finitebrain-semantic-index-v1',
                        lifecycle = CASE WHEN enabled = 1 THEN 'building' ELSE 'disabled' END,
                        active_generation = NULL,
                        last_error_class = NULL
                  WHERE singleton = 1;",
            )
            .map_err(index_error)?;
    }
    Ok(())
}

pub(crate) fn status(connection: &Connection) -> Result<SemanticStatus, CliError> {
    ensure_schema(connection)?;
    let (enabled, stored_lifecycle, active_generation): (bool, String, Option<String>) = connection
        .query_row(
            "SELECT enabled, lifecycle, active_generation
               FROM semantic_settings WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(index_error)?;
    let current_sections = section_count(connection)?;
    let current_vectors = active_generation
        .as_deref()
        .map(|generation| current_vector_count(connection, generation))
        .transpose()?
        .unwrap_or_default();
    let contract = active_generation
        .as_deref()
        .map(|generation| generation_contract(connection, generation))
        .transpose()?
        .flatten();
    let lifecycle = if !enabled {
        SemanticLifecycle::Disabled
    } else {
        match stored_lifecycle.as_str() {
            "failed" => SemanticLifecycle::Failed,
            "stale" => SemanticLifecycle::Stale,
            "building" if active_generation.is_none() => SemanticLifecycle::Building,
            _ if current_sections == 0 => SemanticLifecycle::Ready,
            _ if active_generation.is_none() => SemanticLifecycle::Building,
            _ if contract.is_none() => SemanticLifecycle::Stale,
            _ if current_vectors == current_sections => SemanticLifecycle::Ready,
            _ => SemanticLifecycle::Stale,
        }
    };
    Ok(SemanticStatus {
        enabled,
        lifecycle,
        model: contract.as_ref().map(|contract| contract.model.clone()),
        model_version: contract
            .as_ref()
            .map(|contract| contract.model_version.clone()),
        dimensions: contract.as_ref().map(|contract| contract.dimensions),
        current_sections,
        current_vectors,
    })
}

pub(crate) fn enable(connection: &Connection) -> Result<(), CliError> {
    ensure_schema(connection)?;
    connection
        .execute(
            "UPDATE semantic_settings
                SET enabled = 1, lifecycle = 'building', last_error_class = NULL
              WHERE singleton = 1",
            [],
        )
        .map_err(index_error)?;
    Ok(())
}

pub(crate) fn disable(connection: &Connection) -> Result<(), CliError> {
    ensure_schema(connection)?;
    let transaction = connection.unchecked_transaction().map_err(index_error)?;
    transaction
        .execute("DELETE FROM semantic_vectors", [])
        .map_err(index_error)?;
    transaction
        .execute("DELETE FROM semantic_generations", [])
        .map_err(index_error)?;
    transaction
        .execute(
            "UPDATE semantic_settings
                SET enabled = 0, lifecycle = 'disabled', active_generation = NULL,
                    last_error_class = NULL
              WHERE singleton = 1",
            [],
        )
        .map_err(index_error)?;
    transaction.commit().map_err(index_error)
}

pub(crate) fn record_failure(connection: &Connection, class: &str) -> Result<(), CliError> {
    ensure_schema(connection)?;
    connection
        .execute(
            "UPDATE semantic_settings
                SET lifecycle = CASE WHEN enabled = 1 THEN 'failed' ELSE 'disabled' END,
                    last_error_class = ?1
              WHERE singleton = 1",
            [class],
        )
        .map_err(index_error)?;
    Ok(())
}

pub(crate) fn invalidate_for_section_format(connection: &Connection) -> Result<(), CliError> {
    ensure_schema(connection)?;
    let transaction = connection.unchecked_transaction().map_err(index_error)?;
    transaction
        .execute("DELETE FROM semantic_vectors", [])
        .map_err(index_error)?;
    transaction
        .execute("DELETE FROM semantic_generations", [])
        .map_err(index_error)?;
    transaction
        .execute(
            "UPDATE semantic_settings
                SET active_generation = NULL,
                    lifecycle = CASE WHEN enabled = 1 THEN 'building' ELSE 'disabled' END,
                    last_error_class = NULL
              WHERE singleton = 1",
            [],
        )
        .map_err(index_error)?;
    transaction.commit().map_err(index_error)
}

pub(crate) fn build_generation(
    connection: &Connection,
    provider: &EmbeddingProviderAdapter,
    admission_lock: &File,
    admission_path: &Path,
) -> Result<bool, CliError> {
    ensure_schema(connection)?;
    let current_status = status(connection)?;
    if !current_status.enabled {
        return Ok(false);
    }
    let sections = sections_for_embedding(connection)?;
    if sections.len() > MAX_SEMANTIC_SECTIONS_PER_FOLDER {
        return Err(CliError::SearchIndex(format!(
            "Folder semantic generation exceeds {MAX_SEMANTIC_SECTIONS_PER_FOLDER} Sections"
        )));
    }
    if sections.is_empty() {
        let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(index_error)?;
        transaction
            .execute("DELETE FROM semantic_vectors", [])
            .map_err(index_error)?;
        transaction
            .execute("DELETE FROM semantic_generations", [])
            .map_err(index_error)?;
        transaction
            .execute(
                "UPDATE semantic_settings
                    SET lifecycle = 'ready', active_generation = NULL, last_error_class = NULL
                  WHERE singleton = 1",
                [],
            )
            .map_err(index_error)?;
        transaction.commit().map_err(index_error)?;
        return Ok(false);
    }
    if current_status.lifecycle == SemanticLifecycle::Ready
        && current_status.current_vectors == sections.len()
    {
        return Ok(false);
    }
    let marked_building = connection
        .execute(
            "UPDATE semantic_settings SET lifecycle = 'building', last_error_class = NULL
              WHERE singleton = 1 AND enabled = 1",
            [],
        )
        .map_err(index_error)?;
    if marked_building == 0 {
        return Ok(false);
    }
    connection
        .execute(
            "DELETE FROM semantic_generations
              WHERE complete = 0
                AND generation_id != COALESCE(
                    (SELECT active_generation FROM semantic_settings WHERE singleton = 1), ''
                )",
            [],
        )
        .map_err(index_error)?;

    let content_generation = content_generation_fingerprint(&sections);
    let active = active_generation_contract(connection)?;
    let reusable = active.as_ref().map_or_else(
        || Ok(BTreeMap::new()),
        |contract| reusable_vectors(connection, contract),
    )?;
    let missing = sections
        .iter()
        .filter(|section| {
            reusable
                .get(&(section.page_path.clone(), section.key.clone()))
                .is_none_or(|vector| vector.section_hash != section.section_hash)
        })
        .collect::<Vec<_>>();
    let probe_current_contract = missing.is_empty()
        && active.is_some()
        && matches!(
            current_status.lifecycle,
            SemanticLifecycle::Stale | SemanticLifecycle::Failed
        );
    if missing.is_empty() && !probe_current_contract {
        connection
            .execute(
                "UPDATE semantic_settings SET lifecycle = 'ready', last_error_class = NULL
                  WHERE singleton = 1 AND enabled = 1",
                [],
            )
            .map_err(index_error)?;
        return Ok(false);
    }

    let seed = if probe_current_contract {
        vec![&sections[0]]
    } else {
        missing
            .iter()
            .take(EMBEDDING_REQUEST_CHUNK)
            .copied()
            .collect::<Vec<_>>()
    };
    let Some(first_response) = request_authorized_embeddings(
        connection,
        admission_lock,
        admission_path,
        provider,
        &seed,
        0,
    )?
    else {
        return Ok(false);
    };
    let contract = contract_for_response(&content_generation, &first_response);
    let active_matches = active.as_ref().is_some_and(|active| {
        active.model == contract.model
            && active.model_version == contract.model_version
            && active.dimensions == contract.dimensions
    });
    if probe_current_contract && active_matches {
        let persisted = with_authorized_admission(
            connection,
            admission_lock,
            admission_path,
            || {
                let transaction =
                    Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
                        .map_err(index_error)?;
                if !snapshot_is_current(&transaction, &content_generation)? {
                    transaction
                        .execute(
                            "UPDATE semantic_settings SET lifecycle = 'building' WHERE singleton = 1 AND enabled = 1",
                            [],
                        )
                        .map_err(index_error)?;
                    transaction.commit().map_err(index_error)?;
                    return Ok(());
                }
                transaction
                    .execute(
                        "UPDATE semantic_settings SET lifecycle = 'ready', last_error_class = NULL
                          WHERE singleton = 1 AND enabled = 1",
                        [],
                    )
                    .map_err(index_error)?;
                transaction.commit().map_err(index_error)
            },
        )?;
        if persisted.is_none() {
            return Ok(false);
        }
        return Ok(false);
    }

    let persisted = with_authorized_admission(connection, admission_lock, admission_path, || {
        connection
            .execute(
                "INSERT OR REPLACE INTO semantic_generations (
                        generation_id, model, model_version, dimensions, section_format, complete
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                params![
                    &contract.id,
                    &contract.model,
                    &contract.model_version,
                    contract.dimensions as i64,
                    SEARCH_INDEX_VERSION,
                ],
            )
            .map_err(index_error)?;
        if active_matches {
            copy_reusable_vectors(connection, &contract.id, &sections, &reusable)?;
        }
        store_vectors(connection, &contract.id, &seed, first_response)
    })?;
    if persisted.is_none() {
        return Ok(false);
    }

    let seeded = seed
        .iter()
        .map(|section| (section.page_path.as_str(), section.key.as_str()))
        .collect::<BTreeSet<_>>();
    let remaining = if active_matches {
        missing
            .iter()
            .copied()
            .filter(|section| !seeded.contains(&(section.page_path.as_str(), section.key.as_str())))
            .collect::<Vec<_>>()
    } else {
        sections
            .iter()
            .filter(|section| !seeded.contains(&(section.page_path.as_str(), section.key.as_str())))
            .collect::<Vec<_>>()
    };
    let mut input_offset = seed.len();
    for chunk in remaining.chunks(EMBEDDING_REQUEST_CHUNK) {
        let response = match request_authorized_embeddings(
            connection,
            admission_lock,
            admission_path,
            provider,
            chunk,
            input_offset,
        ) {
            Ok(Some(response)) => response,
            Ok(None) => {
                cleanup_generation(connection, Some(&contract.id))?;
                return Ok(false);
            }
            Err(error) => {
                cleanup_generation(connection, Some(&contract.id))?;
                return Err(error);
            }
        };
        if response.model != contract.model
            || response.model_version != contract.model_version
            || response.dimensions != contract.dimensions
        {
            cleanup_generation(connection, Some(&contract.id))?;
            return Err(CliError::EmbeddingProvider(
                "provider changed model contract during Folder generation".to_owned(),
            ));
        }
        let persisted =
            with_authorized_admission(connection, admission_lock, admission_path, || {
                store_vectors(connection, &contract.id, chunk, response)
            })?;
        if persisted.is_none() {
            return Ok(false);
        }
        input_offset += chunk.len();
    }

    Ok(
        with_authorized_admission(connection, admission_lock, admission_path, || {
            activate_generation(
                connection,
                &contract.id,
                &content_generation,
                sections.len(),
            )
        })?
        .unwrap_or(false),
    )
}

fn activate_generation(
    connection: &Connection,
    generation: &str,
    content_generation: &str,
    section_count: usize,
) -> Result<bool, CliError> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(index_error)?;
    if !snapshot_is_current(&transaction, content_generation)? {
        transaction
            .execute(
                "DELETE FROM semantic_generations WHERE generation_id = ?1 AND complete = 0",
                [generation],
            )
            .map_err(index_error)?;
        transaction
            .execute(
                "UPDATE semantic_settings SET lifecycle = 'building'
                  WHERE singleton = 1 AND enabled = 1",
                [],
            )
            .map_err(index_error)?;
        transaction.commit().map_err(index_error)?;
        return Ok(false);
    }
    let staged: usize = transaction
        .query_row(
            "SELECT count(*) FROM semantic_vectors WHERE generation_id = ?1",
            [generation],
            |row| row.get(0),
        )
        .map_err(index_error)?;
    if staged != section_count {
        transaction
            .execute(
                "DELETE FROM semantic_generations WHERE generation_id = ?1 AND complete = 0",
                [generation],
            )
            .map_err(index_error)?;
        transaction.commit().map_err(index_error)?;
        return Err(CliError::SearchIndex(
            "semantic generation cardinality did not match current Sections".to_owned(),
        ));
    }
    transaction
        .execute(
            "UPDATE semantic_generations SET complete = 1 WHERE generation_id = ?1",
            [generation],
        )
        .map_err(index_error)?;
    let activated = transaction
        .execute(
            "UPDATE semantic_settings
                SET active_generation = ?1, lifecycle = 'ready', last_error_class = NULL
              WHERE singleton = 1 AND enabled = 1",
            [generation],
        )
        .map_err(index_error)?;
    if activated == 0 {
        transaction
            .execute(
                "DELETE FROM semantic_generations WHERE generation_id = ?1",
                [generation],
            )
            .map_err(index_error)?;
        transaction.commit().map_err(index_error)?;
        return Ok(false);
    }
    transaction
        .execute(
            "DELETE FROM semantic_generations WHERE generation_id != ?1",
            [generation],
        )
        .map_err(index_error)?;
    transaction.commit().map_err(index_error)?;
    Ok(true)
}

pub(crate) fn active_contract(
    connection: &Connection,
) -> Result<Option<(String, String, usize)>, CliError> {
    ensure_schema(connection)?;
    connection
        .query_row(
            "SELECT g.model, g.model_version, g.dimensions
               FROM semantic_settings s
               JOIN semantic_generations g ON g.generation_id = s.active_generation
              WHERE s.singleton = 1 AND s.enabled = 1 AND g.complete = 1
                AND g.section_format = ?1",
            [SEARCH_INDEX_VERSION],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(index_error)
}

pub(crate) fn semantic_candidates(
    connection: &Connection,
    query: &EmbeddingProviderResponse,
    limit: usize,
) -> Result<Vec<SemanticCandidate>, CliError> {
    ensure_schema(connection)?;
    let Some((model, model_version, dimensions)) = active_contract(connection)? else {
        return Ok(Vec::new());
    };
    if query.model != model
        || query.model_version != model_version
        || query.dimensions != dimensions
        || query.vectors.len() != 1
    {
        connection
            .execute(
                "UPDATE semantic_settings SET lifecycle = 'stale' WHERE singleton = 1",
                [],
            )
            .map_err(index_error)?;
        return Ok(Vec::new());
    }
    let query_vector = &query.vectors[0].embedding;
    let active_generation: String = connection
        .query_row(
            "SELECT active_generation FROM semantic_settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(index_error)?;
    let mut statement = connection
        .prepare(
            "SELECT s.section_key, s.page_path, s.page_title, s.heading_ancestry,
                    s.heading, s.disposition, v.vector
               FROM semantic_vectors v
               JOIN sections s
                 ON s.section_key = v.section_key
                AND s.page_path = v.page_path
                AND s.section_hash = v.section_hash
              WHERE v.generation_id = ?1",
        )
        .map_err(index_error)?;
    let rows = statement
        .query_map([active_generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Vec<u8>>(6)?,
            ))
        })
        .map_err(index_error)?;
    let mut ranked = Vec::<RankedSemanticCandidate>::with_capacity(limit);
    let mut scanned = 0_usize;
    for row in rows {
        let (section_key, page_path, page_title, ancestry, heading, disposition, vector) =
            row.map_err(index_error)?;
        scanned = scanned.saturating_add(1);
        if scanned > MAX_SEMANTIC_SECTIONS_PER_FOLDER {
            return Err(CliError::SearchIndex(format!(
                "semantic candidate scan exceeds {MAX_SEMANTIC_SECTIONS_PER_FOLDER} Sections"
            )));
        }
        let vector = decode_vector(&vector, dimensions)?;
        let Some(similarity) = cosine_similarity(query_vector, &vector) else {
            continue;
        };
        if similarity <= 0.0 {
            continue;
        }
        let candidate = RankedSemanticCandidate {
            section_key,
            page_path,
            page_title,
            heading_ancestry: ancestry,
            heading,
            disposition,
            similarity,
        };
        if ranked.len() < limit {
            ranked.push(candidate);
        } else if let Some((worst_index, worst)) = ranked
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| semantic_rank_cmp(left, right))
            && semantic_rank_cmp(&candidate, worst).is_gt()
        {
            ranked[worst_index] = candidate;
        }
    }
    ranked.sort_by(|left, right| semantic_rank_cmp(right, left));
    let mut candidates = Vec::with_capacity(ranked.len());
    for candidate in ranked {
        let body = connection
            .query_row(
                "SELECT body FROM sections WHERE section_key = ?1 AND page_path = ?2",
                params![&candidate.section_key, &candidate.page_path],
                |row| row.get::<_, String>(0),
            )
            .map_err(index_error)?;
        candidates.push(SemanticCandidate {
            section_key: candidate.section_key,
            page_path: candidate.page_path,
            page_title: candidate.page_title,
            heading_ancestry: serde_json::from_str(&candidate.heading_ancestry).unwrap_or_default(),
            heading: candidate.heading,
            excerpt: excerpt(&body),
            disposition: candidate.disposition,
            similarity: candidate.similarity,
        });
    }
    Ok(candidates)
}

fn semantic_rank_cmp(
    left: &RankedSemanticCandidate,
    right: &RankedSemanticCandidate,
) -> std::cmp::Ordering {
    left.similarity
        .total_cmp(&right.similarity)
        .then_with(|| right.page_path.cmp(&left.page_path))
        .then_with(|| right.section_key.cmp(&left.section_key))
}

pub(crate) fn active_vector_count(connection: &Connection) -> Result<usize, CliError> {
    ensure_schema(connection)?;
    connection
        .query_row(
            "SELECT count(*)
               FROM semantic_vectors v
               JOIN semantic_settings s ON s.active_generation = v.generation_id
               JOIN semantic_generations g ON g.generation_id = v.generation_id
              WHERE s.singleton = 1 AND s.enabled = 1 AND g.complete = 1
                AND g.section_format = ?1",
            [SEARCH_INDEX_VERSION],
            |row| row.get(0),
        )
        .map_err(index_error)
}

struct SharedAdmission<'a>(&'a File);

impl SharedAdmission<'_> {
    fn acquire(file: &File) -> Result<SharedAdmission<'_>, CliError> {
        flock(file, FlockOperation::LockShared)
            .map_err(|error| CliError::SearchIndex(format!("semantic admission lock: {error}")))?;
        Ok(SharedAdmission(file))
    }
}

impl Drop for SharedAdmission<'_> {
    fn drop(&mut self) {
        let _ = flock(self.0, FlockOperation::Unlock);
    }
}

fn request_authorized_embeddings(
    connection: &Connection,
    admission_lock: &File,
    admission_path: &Path,
    provider: &EmbeddingProviderAdapter,
    sections: &[&SectionForEmbedding],
    input_offset: usize,
) -> Result<Option<EmbeddingProviderResponse>, CliError> {
    with_authorized_admission(connection, admission_lock, admission_path, || {
        request_embeddings(provider, sections, input_offset)
    })
}

fn with_authorized_admission<T>(
    connection: &Connection,
    admission_lock: &File,
    admission_path: &Path,
    action: impl FnOnce() -> Result<T, CliError>,
) -> Result<Option<T>, CliError> {
    if admission_path.parent().is_some_and(|directory| {
        directory.join("semantic-revoking").exists() || directory.join("access-revoked").exists()
    }) {
        return Ok(None);
    }
    let _admission = SharedAdmission::acquire(admission_lock)?;
    if !lock_matches_path(admission_lock, admission_path) {
        return Ok(None);
    }
    if admission_path.parent().is_some_and(|directory| {
        directory.join("semantic-revoking").exists() || directory.join("access-revoked").exists()
    }) {
        return Ok(None);
    }
    let enabled: bool = connection
        .query_row(
            "SELECT enabled FROM semantic_settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(index_error)?;
    if !enabled {
        return Ok(None);
    }
    action().map(Some)
}

pub(crate) fn lock_matches_path(file: &File, path: &Path) -> bool {
    let Ok(open_metadata) = file.metadata() else {
        return false;
    };
    let Ok(path_metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    open_metadata.nlink() > 0
        && path_metadata.is_file()
        && !path_metadata.file_type().is_symlink()
        && open_metadata.dev() == path_metadata.dev()
        && open_metadata.ino() == path_metadata.ino()
}

fn request_embeddings(
    provider: &EmbeddingProviderAdapter,
    sections: &[&SectionForEmbedding],
    input_offset: usize,
) -> Result<EmbeddingProviderResponse, CliError> {
    let inputs = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let text = if section.body.trim().is_empty() {
                section
                    .heading
                    .as_deref()
                    .unwrap_or(section.page_title.as_str())
            } else {
                section.body.as_str()
            };
            EmbeddingProviderInput::section(
                format!("input-{}", input_offset + index),
                &section.page_title,
                section.heading_ancestry.clone(),
                text,
            )
        })
        .collect::<Vec<_>>();
    provider.embed(&inputs)
}

fn store_vectors(
    connection: &Connection,
    generation: &str,
    sections: &[&SectionForEmbedding],
    response: EmbeddingProviderResponse,
) -> Result<(), CliError> {
    let transaction = connection.unchecked_transaction().map_err(index_error)?;
    for (section, vector) in sections.iter().zip(response.vectors) {
        transaction
            .execute(
                "INSERT OR REPLACE INTO semantic_vectors (
                    generation_id, section_key, page_path, section_hash, vector
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    generation,
                    &section.key,
                    &section.page_path,
                    &section.section_hash,
                    encode_vector(&vector.embedding),
                ],
            )
            .map_err(index_error)?;
    }
    transaction.commit().map_err(index_error)
}

fn active_generation_contract(
    connection: &Connection,
) -> Result<Option<GenerationContract>, CliError> {
    let generation: Option<String> = connection
        .query_row(
            "SELECT active_generation FROM semantic_settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(index_error)?;
    generation
        .as_deref()
        .map(|generation| generation_contract(connection, generation))
        .transpose()
        .map(Option::flatten)
}

fn reusable_vectors(
    connection: &Connection,
    active: &GenerationContract,
) -> Result<BTreeMap<(String, String), ReusableVector>, CliError> {
    let mut statement = connection
        .prepare(
            "SELECT page_path, section_key, section_hash, vector
               FROM semantic_vectors WHERE generation_id = ?1",
        )
        .map_err(index_error)?;
    let rows = statement
        .query_map([&active.id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(index_error)?;
    let mut reusable = BTreeMap::new();
    for row in rows {
        let (page_path, section_key, section_hash, vector) = row.map_err(index_error)?;
        if decode_vector(&vector, active.dimensions).is_ok() {
            reusable.insert(
                (page_path, section_key),
                ReusableVector {
                    section_hash,
                    vector,
                },
            );
        }
    }
    Ok(reusable)
}

fn copy_reusable_vectors(
    connection: &Connection,
    generation: &str,
    sections: &[SectionForEmbedding],
    reusable: &BTreeMap<(String, String), ReusableVector>,
) -> Result<(), CliError> {
    let transaction = connection.unchecked_transaction().map_err(index_error)?;
    for section in sections {
        let Some(vector) = reusable.get(&(section.page_path.clone(), section.key.clone())) else {
            continue;
        };
        if vector.section_hash != section.section_hash {
            continue;
        }
        transaction
            .execute(
                "INSERT INTO semantic_vectors (
                    generation_id, section_key, page_path, section_hash, vector
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    generation,
                    &section.key,
                    &section.page_path,
                    &section.section_hash,
                    &vector.vector,
                ],
            )
            .map_err(index_error)?;
    }
    transaction.commit().map_err(index_error)
}

fn snapshot_is_current(
    connection: &Connection,
    expected_fingerprint: &str,
) -> Result<bool, CliError> {
    Ok(
        content_generation_fingerprint(&sections_for_embedding(connection)?)
            == expected_fingerprint,
    )
}

fn sections_for_embedding(connection: &Connection) -> Result<Vec<SectionForEmbedding>, CliError> {
    let mut statement = connection
        .prepare(
            "SELECT section_key, page_path, page_title, heading_ancestry, heading, body, section_hash
               FROM sections ORDER BY page_path, section_key",
        )
        .map_err(index_error)?;
    let rows = statement
        .query_map([], |row| {
            let ancestry: String = row.get(3)?;
            Ok(SectionForEmbedding {
                key: row.get(0)?,
                page_path: row.get(1)?,
                page_title: row.get(2)?,
                heading_ancestry: serde_json::from_str(&ancestry).unwrap_or_default(),
                heading: row.get(4)?,
                body: row.get(5)?,
                section_hash: row.get(6)?,
            })
        })
        .map_err(index_error)?;
    rows.collect::<Result<_, _>>().map_err(index_error)
}

fn section_count(connection: &Connection) -> Result<usize, CliError> {
    connection
        .query_row("SELECT count(*) FROM sections", [], |row| row.get(0))
        .map_err(index_error)
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, CliError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info('{table}')"))
        .map_err(index_error)?;
    let rows = statement
        .query_map([], |row| row.get(1))
        .map_err(index_error)?;
    rows.collect::<Result<_, _>>().map_err(index_error)
}

fn current_vector_count(connection: &Connection, generation: &str) -> Result<usize, CliError> {
    connection
        .query_row(
            "SELECT count(*)
               FROM semantic_vectors v
               JOIN sections s
                 ON s.section_key = v.section_key
                AND s.page_path = v.page_path
                AND s.section_hash = v.section_hash
              WHERE v.generation_id = ?1",
            [generation],
            |row| row.get(0),
        )
        .map_err(index_error)
}

fn generation_contract(
    connection: &Connection,
    generation: &str,
) -> Result<Option<GenerationContract>, CliError> {
    connection
        .query_row(
            "SELECT generation_id, model, model_version, dimensions
               FROM semantic_generations
              WHERE generation_id = ?1 AND complete = 1 AND section_format = ?2",
            params![generation, SEARCH_INDEX_VERSION],
            |row| {
                Ok(GenerationContract {
                    id: row.get(0)?,
                    model: row.get(1)?,
                    model_version: row.get(2)?,
                    dimensions: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(index_error)
}

fn contract_for_response(
    content_generation: &str,
    response: &EmbeddingProviderResponse,
) -> GenerationContract {
    let identity = format!(
        "{content_generation}\n{}\n{}\n{}",
        response.model, response.model_version, response.dimensions
    );
    GenerationContract {
        id: format!("generation-{}", &sha256_hex(identity.as_bytes())[..24]),
        model: response.model.clone(),
        model_version: response.model_version.clone(),
        dimensions: response.dimensions,
    }
}

fn content_generation_fingerprint(sections: &[SectionForEmbedding]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SEMANTIC_SCHEMA_VERSION.as_bytes());
    hasher.update(SEARCH_INDEX_VERSION.as_bytes());
    for section in sections {
        for value in [&section.page_path, &section.key, &section.section_hash] {
            hasher.update(value.len().to_le_bytes());
            hasher.update(value.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn cleanup_generation(connection: &Connection, generation: Option<&str>) -> Result<(), CliError> {
    if let Some(generation) = generation {
        connection
            .execute(
                "DELETE FROM semantic_generations WHERE generation_id = ?1 AND complete = 0",
                [generation],
            )
            .map_err(index_error)?;
    }
    Ok(())
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_vector(bytes: &[u8], dimensions: usize) -> Result<Vec<f32>, CliError> {
    if bytes.len() != dimensions.saturating_mul(std::mem::size_of::<f32>()) {
        return Err(CliError::SearchIndex(
            "semantic vector byte length did not match its generation".to_owned(),
        ));
    }
    let vector = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(CliError::SearchIndex(
            "semantic vector contained a non-finite value".to_owned(),
        ));
    }
    Ok(vector)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    let denominator = left_norm * right_norm;
    if denominator <= f32::EPSILON {
        None
    } else {
        let similarity = dot / denominator;
        similarity.is_finite().then_some(similarity)
    }
}

fn excerpt(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let excerpt = chars.by_ref().take(240).collect::<String>();
    if chars.next().is_some() {
        format!("{excerpt} …")
    } else {
        excerpt
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn index_error(error: rusqlite::Error) -> CliError {
    CliError::SearchIndex(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_uses_known_orthogonal_vectors() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), None);
    }

    #[test]
    fn vector_storage_round_trips_without_changing_values() {
        let expected = vec![0.25, -0.5, 1.0];
        let encoded = encode_vector(&expected);
        assert_eq!(decode_vector(&encoded, 3).unwrap(), expected);
        assert!(decode_vector(&encoded, 4).is_err());
    }

    #[test]
    fn admission_rejects_an_unlinked_and_recreated_lock_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("semantic-admission.lock");
        File::create(&path).unwrap();
        let original = File::open(&path).unwrap();
        let _admission = SharedAdmission::acquire(&original).unwrap();

        std::fs::remove_file(&path).unwrap();
        File::create(&path).unwrap();

        assert!(!lock_matches_path(&original, &path));
    }

    #[test]
    fn incompatible_section_format_is_never_active_and_is_invalidated() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sections (
                    section_key TEXT, page_path TEXT, section_hash TEXT
                 );
                 INSERT INTO sections VALUES ('section', 'page.md', 'hash');",
            )
            .unwrap();
        ensure_schema(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO semantic_generations (
                    generation_id, model, model_version, dimensions, section_format, complete
                 ) VALUES ('old-generation', 'model', 'v1', 2, 'old-section-format', 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO semantic_vectors (
                    generation_id, section_key, page_path, section_hash, vector
                 ) VALUES ('old-generation', 'section', 'page.md', 'hash', ?1)",
                [encode_vector(&[1.0, 0.0])],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE semantic_settings
                    SET active_generation = 'old-generation', lifecycle = 'ready'",
                [],
            )
            .unwrap();

        assert!(active_contract(&connection).unwrap().is_none());
        assert_eq!(
            status(&connection).unwrap().lifecycle,
            SemanticLifecycle::Stale
        );
        invalidate_for_section_format(&connection).unwrap();
        let (generation_count, active, lifecycle): (usize, Option<String>, String) = connection
            .query_row(
                "SELECT
                    (SELECT count(*) FROM semantic_generations),
                    active_generation,
                    lifecycle
                   FROM semantic_settings WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(generation_count, 0);
        assert!(active.is_none());
        assert_eq!(lifecycle, "building");
    }
}
