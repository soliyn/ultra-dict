//! The Library: the one place that owns dictionaries, the headword index,
//! search, articles, resources and history (`docs/spec-v1.md`).
//!
//! Everything is persisted in one SQLite database. Callers (Tauri commands,
//! tests) use only this module's types; SQLite and the format layer stay
//! behind it.

mod fold;

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::format::{self, Dictionary, FormatError};
use fold::fold;

/// Schema version written to `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;

/// The user's dictionary collection and its index.
pub struct Library {
    db: Connection,
}

impl Library {
    /// Opens the library database at `path`, creating the file, its parent
    /// directories and the schema if they don't exist yet.
    pub fn open(path: &Path) -> Result<Self, LibraryError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        let mut library = Self { db };
        library.migrate()?;
        Ok(library)
    }

    fn migrate(&mut self) -> Result<(), LibraryError> {
        let version: i64 = self.db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                let tx = self.db.transaction()?;
                tx.execute_batch(include_str!("schema.sql"))?;
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                tx.commit()?;
                Ok(())
            }
            SCHEMA_VERSION => Ok(()),
            newer => Err(LibraryError::NewerSchema(newer)),
        }
    }

    /// Adds the dictionary file at `path` and indexes it.
    ///
    /// The file is referenced in place. Adding a path that is already in the
    /// library changes nothing and returns the existing record.
    ///
    /// A file that cannot be opened or indexed is still recorded, with
    /// [`Status::Error`], so the user sees why; that is not an `Err`. Indexing
    /// is atomic: a dictionary has either all of its headwords or none.
    /// `Err` means the library itself (the database) failed.
    pub fn add_file(&mut self, path: &Path) -> Result<DictionaryInfo, LibraryError> {
        let path = std::path::absolute(path)?;
        let path_text = path
            .to_str()
            .ok_or_else(|| LibraryError::PathNotUnicode(path.clone()))?;
        if let Some(existing) = self.find_by_path(path_text)? {
            return Ok(existing);
        }

        let stamp = FileStamp::read(&path);
        let (title, failure) = match format::open(&path) {
            Ok(dict) => {
                let title = dict.metadata().title.clone();
                match self.insert_indexed(path_text, &title, stamp, dict.as_ref())? {
                    Ok(id) => return self.get(id),
                    Err(error) => (title, error),
                }
            }
            Err(error) => (format::display_title(None, &path), error),
        };
        let id = self.insert_failed(path_text, &title, stamp, &failure.to_string())?;
        self.get(id)
    }

    /// Every dictionary in library order.
    pub fn dictionaries(&self) -> Result<Vec<DictionaryInfo>, LibraryError> {
        let mut stmt = self
            .db
            .prepare_cached(&format!("{SELECT_INFO} ORDER BY d.position, d.id"))?;
        let rows = stmt.query_map([], DictionaryInfo::from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn get(&self, id: DictionaryId) -> Result<DictionaryInfo, LibraryError> {
        let mut stmt = self.db.prepare_cached(&format!("{SELECT_INFO} WHERE d.id = ?1"))?;
        Ok(stmt.query_row([id.0], DictionaryInfo::from_row)?)
    }

    fn find_by_path(&self, path: &str) -> Result<Option<DictionaryInfo>, LibraryError> {
        let mut stmt = self
            .db
            .prepare_cached(&format!("{SELECT_INFO} WHERE d.path = ?1"))?;
        Ok(stmt.query_row([path], DictionaryInfo::from_row).optional()?)
    }

    /// Records the dictionary and all its headwords in one transaction.
    ///
    /// The outer `Err` is a database failure; the inner one a format error
    /// while reading headwords, after which nothing has been written.
    fn insert_indexed(
        &mut self,
        path: &str,
        title: &str,
        stamp: Option<FileStamp>,
        dict: &dyn Dictionary,
    ) -> Result<Result<DictionaryId, FormatError>, LibraryError> {
        let tx = self.db.transaction()?;
        let id = insert_dictionary(&tx, path, title, stamp, "ready", None)?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO headwords (dictionary_id, headword, folded, locator)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for headword in dict.headwords() {
                let headword = match headword {
                    Ok(headword) => headword,
                    // Dropping `tx` rolls everything back.
                    Err(error) => return Ok(Err(error)),
                };
                insert.execute(params![
                    id.0,
                    headword.text,
                    fold(&headword.text),
                    // Bit-preserving: locators round-trip through i64.
                    headword.locator.to_raw() as i64,
                ])?;
            }
        }
        tx.commit()?;
        Ok(Ok(id))
    }

    fn insert_failed(
        &mut self,
        path: &str,
        title: &str,
        stamp: Option<FileStamp>,
        message: &str,
    ) -> Result<DictionaryId, LibraryError> {
        let tx = self.db.transaction()?;
        let id = insert_dictionary(&tx, path, title, stamp, "error", Some(message))?;
        tx.commit()?;
        Ok(id)
    }
}

/// Inserts a dictionary row at the end of the library order.
fn insert_dictionary(
    tx: &Transaction<'_>,
    path: &str,
    title: &str,
    stamp: Option<FileStamp>,
    status: &str,
    error: Option<&str>,
) -> Result<DictionaryId, LibraryError> {
    tx.execute(
        "INSERT INTO dictionaries
             (path, title, position, enabled, file_size, file_mtime_ns, status, error)
         VALUES
             (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM dictionaries), 1, ?3, ?4, ?5, ?6)",
        params![
            path,
            title,
            stamp.map(|s| s.size as i64),
            stamp.map(|s| s.modified_ns()),
            status,
            error,
        ],
    )?;
    Ok(DictionaryId(tx.last_insert_rowid()))
}

const SELECT_INFO: &str = "
    SELECT d.id, d.path, d.title, d.position, d.enabled, d.file_size, d.file_mtime_ns,
           d.status, d.error,
           (SELECT COUNT(*) FROM headwords h WHERE h.dictionary_id = d.id)
    FROM dictionaries d";

/// Identifies a dictionary within one library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DictionaryId(i64);

impl fmt::Display for DictionaryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A dictionary as the library lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryInfo {
    pub id: DictionaryId,
    /// Absolute path of the dictionary file.
    pub path: PathBuf,
    /// Display name.
    pub title: String,
    /// Place in the library order (0 is first).
    pub position: u32,
    pub enabled: bool,
    /// Size and modification time of the file when it was indexed, or
    /// `None` if the file could not be read.
    pub file: Option<FileStamp>,
    pub status: Status,
    /// Number of indexed headwords (0 unless [`Status::Ready`]).
    pub headword_count: u64,
}

impl DictionaryInfo {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let size: Option<i64> = row.get(5)?;
        let mtime_ns: Option<i64> = row.get(6)?;
        let status: String = row.get(7)?;
        let error: Option<String> = row.get(8)?;
        Ok(Self {
            id: DictionaryId(row.get(0)?),
            path: PathBuf::from(row.get::<_, String>(1)?),
            title: row.get(2)?,
            position: row.get(3)?,
            enabled: row.get(4)?,
            file: size.zip(mtime_ns).map(|(size, ns)| FileStamp {
                size: size as u64,
                modified: UNIX_EPOCH + Duration::from_nanos(ns as u64),
            }),
            status: match status.as_str() {
                "ready" => Status::Ready,
                _ => Status::Error(error.unwrap_or(status)),
            },
            headword_count: row.get::<_, i64>(9)? as u64,
        })
    }
}

/// Where a dictionary stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Indexed and searchable.
    Ready,
    /// Could not be opened or indexed; the message says why
    /// (e.g. `"unsupported: needs registration key"`).
    Error(String),
}

/// A file's size and modification time, used to notice changes on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub modified: SystemTime,
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            size: meta.len(),
            modified: meta.modified().ok()?,
        })
    }

    /// Nanoseconds since the Unix epoch (0 for earlier times).
    fn modified_ns(self) -> i64 {
        self.modified
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
    }
}

/// The library itself failed (as opposed to one dictionary in it).
#[derive(Debug)]
pub enum LibraryError {
    /// The database could not be read or written.
    Database(rusqlite::Error),
    /// A filesystem operation failed.
    Io(io::Error),
    /// The database was written by a newer version of the app.
    NewerSchema(i64),
    /// The path cannot be stored because it is not valid Unicode.
    PathNotUnicode(PathBuf),
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(f, "library database error: {e}"),
            Self::Io(e) => write!(f, "library file error: {e}"),
            Self::NewerSchema(v) => write!(
                f,
                "the library was created by a newer version of the app (schema {v})"
            ),
            Self::PathNotUnicode(p) => {
                write!(f, "unsupported file path (not valid Unicode): {}", p.display())
            }
        }
    }
}

impl std::error::Error for LibraryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for LibraryError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}

impl From<io::Error> for LibraryError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
