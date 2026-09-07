//! Normalized SQLite storage for the finite chat server (storage rewrite).
//!
//! [`Store`] owns one writer connection (every mutation runs inside a
//! `BEGIN IMMEDIATE` transaction) and a small pool of `query_only` read
//! connections. [`delivery::SqlDelivery`] implements the upstream
//! [`finitechat_delivery::HttpDelivery`] contract directly against the
//! normalized tables in [`schema`], with sequences allocated from the route
//! head under the write lock.

pub(crate) mod delivery;
pub(crate) mod metadata;
pub(crate) mod room_state;
pub(crate) mod schema;

pub(crate) use delivery::SqlDelivery;

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use finitechat_delivery::HttpServerError;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::DurableStoreError;

/// Number of pooled read connections for file-backed stores.
const READ_POOL_SIZE: usize = 4;

/// Environment variable overriding the writer's `PRAGMA synchronous` level.
/// Accepted values: `NORMAL` (default) and `FULL`.
///
/// `NORMAL` never corrupts a WAL database: a power loss can at most lose the
/// tail of recently committed transactions, not tear the file. The
/// compensating control for that durability window is continuous replication.
const SYNCHRONOUS_ENV: &str = "FINITECHAT_SQLITE_SYNCHRONOUS";

/// Error produced inside a [`Store::write`] or [`Store::read`] closure.
///
/// `Sqlite` is an infrastructure failure; `Domain` is a delivery-contract
/// rejection; `Store` widens a durable-store failure (e.g. room-state
/// checkpoint divergence surfaced during boot derivation). Any of them
/// aborts the surrounding transaction (the write path rolls back), but they
/// must stay distinct so the server layer can turn `Domain` into its
/// existing HTTP error mapping while the others surface as a 500 or a
/// refused boot.
#[derive(Debug)]
pub(crate) enum StoreTxError {
    Sqlite(rusqlite::Error),
    Domain(HttpServerError),
    Store(DurableStoreError),
}

impl From<rusqlite::Error> for StoreTxError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<HttpServerError> for StoreTxError {
    fn from(error: HttpServerError) -> Self {
        Self::Domain(error)
    }
}

impl From<DurableStoreError> for StoreTxError {
    fn from(error: DurableStoreError) -> Self {
        Self::Store(error)
    }
}

/// Outcome of a [`Store::write`] or [`Store::read`] call: the closure's
/// [`StoreTxError`] with infrastructure failures widened to
/// [`DurableStoreError`]. `Domain` carries a delivery-contract rejection the
/// eventual server layer maps to its existing HTTP errors.
#[derive(Debug)]
pub(crate) enum StoreWriteError {
    Store(DurableStoreError),
    Domain(HttpServerError),
}

impl From<DurableStoreError> for StoreWriteError {
    fn from(error: DurableStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<rusqlite::Error> for StoreWriteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(DurableStoreError::Sqlite(error))
    }
}

impl From<StoreTxError> for StoreWriteError {
    fn from(error: StoreTxError) -> Self {
        match error {
            StoreTxError::Sqlite(error) => Self::Store(DurableStoreError::Sqlite(error)),
            StoreTxError::Domain(error) => Self::Domain(error),
            StoreTxError::Store(error) => Self::Store(error),
        }
    }
}

/// One SQLite database with a single writer and a round-robin read pool.
///
/// In-memory stores keep the pool empty and route reads through the writer
/// connection instead: a plain `:memory:` database is private to its
/// connection, and sharing one via `cache=shared` URIs would trade the
/// simplicity of this fallback for shared-cache table locking. File-backed
/// stores get the real pool.
pub(crate) struct Store {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

impl Store {
    /// Open (creating if needed) a file-backed store with WAL journaling and
    /// a `query_only` read pool.
    pub(crate) fn open_file(path: impl AsRef<Path>) -> Result<Store, DurableStoreError> {
        let path = path.as_ref();
        let writer = Connection::open(path)?;
        configure_common(&writer)?;
        // WAL is a property of the database file; setting it on the writer
        // before the readers open covers every connection.
        writer.execute_batch("PRAGMA journal_mode = WAL;")?;
        configure_writer_synchronous(&writer)?;
        schema::migrate_schema(&writer)?;

        let mut readers = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let reader = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            configure_common(&reader)?;
            reader.execute_batch("PRAGMA query_only = ON;")?;
            readers.push(Mutex::new(reader));
        }

        Ok(Store {
            writer: Mutex::new(writer),
            readers,
            next_reader: AtomicUsize::new(0),
        })
    }

    /// Open a private in-memory store.
    ///
    /// The read pool stays empty; [`Store::read`] falls back to the writer
    /// connection (see the type-level docs for why). WAL and `synchronous`
    /// are meaningless without a file, so only the common PRAGMAs apply.
    /// `HttpServerState::new` (volatile tests/dev servers) and the
    /// in-memory conformance harness use this; durable servers always open
    /// a file.
    pub(crate) fn open_in_memory() -> Result<Store, DurableStoreError> {
        let writer = Connection::open_in_memory()?;
        configure_common(&writer)?;
        schema::migrate_schema(&writer)?;
        Ok(Store {
            writer: Mutex::new(writer),
            readers: Vec::new(),
            next_reader: AtomicUsize::new(0),
        })
    }

    /// Run one mutation inside a `BEGIN IMMEDIATE` transaction on the writer
    /// connection: commits on `Ok`, rolls back on any `Err`.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&mut rusqlite::Transaction) -> Result<T, StoreTxError>,
    ) -> Result<T, StoreWriteError> {
        let mut conn = self
            .writer
            .lock()
            .expect("SQLite writer mutex must not be poisoned");
        let mut tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match f(&mut tx) {
            Ok(value) => {
                tx.commit()?;
                Ok(value)
            }
            Err(error) => {
                // Dropping the transaction rolls it back.
                drop(tx);
                Err(error.into())
            }
        }
    }

    /// Run one read-only query on a pooled `query_only` connection
    /// (round-robin), or on the writer connection for in-memory stores.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreTxError>,
    ) -> Result<T, StoreWriteError> {
        if self.readers.is_empty() {
            let conn = self
                .writer
                .lock()
                .expect("SQLite writer mutex must not be poisoned");
            return f(&conn).map_err(StoreWriteError::from);
        }
        let index = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        let conn = self.readers[index]
            .lock()
            .expect("SQLite reader mutex must not be poisoned");
        f(&conn).map_err(StoreWriteError::from)
    }
}

/// PRAGMAs applied to every connection, writer and reader alike.
fn configure_common(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
}

/// Apply the writer's `synchronous` level: `NORMAL` unless
/// [`SYNCHRONOUS_ENV`] says `FULL`.
fn configure_writer_synchronous(conn: &Connection) -> Result<(), DurableStoreError> {
    let level = match std::env::var(SYNCHRONOUS_ENV) {
        Ok(value) if value.eq_ignore_ascii_case("FULL") => "FULL",
        Ok(value) if value.eq_ignore_ascii_case("NORMAL") => "NORMAL",
        Ok(value) => {
            return Err(DurableStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{SYNCHRONOUS_ENV} must be FULL or NORMAL, got {value:?}"),
            )));
        }
        Err(_) => "NORMAL",
    };
    conn.execute_batch(&format!("PRAGMA synchronous = {level};"))?;
    Ok(())
}

/// Store an unsigned sequence/epoch/timestamp value as SQLite `INTEGER`.
/// Real values fit comfortably in `i64`; overflow is an infrastructure error.
pub(crate) fn u64_to_db(value: u64) -> Result<i64, rusqlite::Error> {
    i64::try_from(value).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

/// Read an unsigned sequence/epoch/timestamp value back from SQLite `INTEGER`.
pub(crate) fn db_to_u64(value: i64) -> Result<u64, rusqlite::Error> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
