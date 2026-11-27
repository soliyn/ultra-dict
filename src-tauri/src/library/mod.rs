//! The Library: the one place that owns dictionaries, the headword index,
//! search, articles, resources and history (`docs/spec-v1.md`).
//!
//! Everything is persisted in one SQLite database. Callers (Tauri commands,
//! tests) use only this module's types; SQLite and the format layer stay
//! behind it.

mod fold;
mod resource;

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::format::{self, Dictionary, EntryLocator, FormatError};
use fold::fold;

/// Schema version written to `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;

/// The user's dictionary collection and its index.
pub struct Library {
    db: Connection,
    /// Dictionary readers opened so far, for articles and resources.
    /// Startup never opens them; each is opened on first use.
    readers: Mutex<HashMap<DictionaryId, Arc<dyn Dictionary>>>,
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
        let mut library = Self {
            db,
            readers: Mutex::default(),
        };
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
                    Ok(id) => {
                        // Already open: keep it for the articles to come.
                        self.lock_readers().insert(id, Arc::from(dict));
                        return self.get(id);
                    }
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

    /// Headwords whose folded form starts with the folded `query`, across
    /// every enabled, ready dictionary: one row per distinct spelling.
    ///
    /// A headword spelled exactly like `query` comes first. The rest are in
    /// case- and diacritic-insensitive order; spellings that fold alike are
    /// ordered by code point (`Polish` before `polish`). An empty query
    /// matches everything, paging through the whole index from the start.
    pub fn search(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<SearchPage, LibraryError> {
        let folded = fold(query);
        // Every key starting with `folded` sorts below this bound: U+10FFFF
        // is the largest code point (and a noncharacter, absent from text).
        let upper = format!("{folded}\u{10FFFF}");

        let exact = !query.is_empty()
            && self
                .db
                .prepare_cached(&format!(
                    "SELECT EXISTS (SELECT 1 FROM headwords h
                     WHERE h.folded = ?1 AND h.headword = ?2 AND {SEARCHABLE})"
                ))?
                .query_row(params![folded, query], |row| row.get::<_, bool>(0))?;

        // The exact match is row 0; the other matches follow in index order,
        // which needs no sort however many rows match.
        let mut items = Vec::with_capacity(limit.min(1000));
        let (rest_offset, rest_limit) = match (exact, offset) {
            (true, 0) => {
                if limit > 0 {
                    items.push(query.to_owned());
                }
                (0, limit.saturating_sub(1))
            }
            (true, _) => (offset - 1, limit),
            (false, _) => (offset, limit),
        };

        // One row per spelling in the folded prefix range `?1..?2`, except
        // the exact spelling `?3`, grouped in index order.
        let other_matches = format!(
            "FROM headwords h
             WHERE h.folded >= ?1 AND h.folded < ?2 AND h.headword <> ?3 AND {SEARCHABLE}
             GROUP BY h.folded, h.headword"
        );
        let rest_count: i64 = self
            .db
            .prepare_cached(&format!("SELECT COUNT(*) FROM (SELECT 1 {other_matches})"))?
            .query_row(params![folded, upper, query], |row| row.get(0))?;

        let mut page = self.db.prepare_cached(&format!(
            "SELECT h.headword {other_matches} ORDER BY h.folded, h.headword LIMIT ?4 OFFSET ?5"
        ))?;
        let rows = page.query_map(
            params![folded, upper, query, to_sql_int(rest_limit), to_sql_int(rest_offset)],
            |row| row.get::<_, String>(0),
        )?;
        for row in rows {
            items.push(row?);
        }

        Ok(SearchPage {
            items,
            total_count: rest_count as u64 + u64::from(exact),
        })
    }

    /// Every entry for exactly `headword` (case and diacritics significant),
    /// one section per enabled, ready dictionary that has it, in library
    /// order. Each section lists its entries in file order.
    ///
    /// Entries are raw HTML; redirects (`@@@LINK=…`) are returned as they
    /// are. A headword no dictionary has gives an article with no sections.
    pub fn article(&self, headword: &str) -> Result<Article, LibraryError> {
        let mut stmt = self.db.prepare_cached(
            "SELECT d.id, d.title, d.path, h.locator
             FROM headwords h JOIN dictionaries d ON d.id = h.dictionary_id
             WHERE h.folded = ?1 AND h.headword = ?2 AND d.enabled AND d.status = 'ready'
             ORDER BY d.position, d.id, h.locator",
        )?;
        let rows = stmt.query_map(params![fold(headword), headword], |row| {
            Ok((
                DictionaryId(row.get(0)?),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                EntryLocator::from_raw(row.get::<_, i64>(3)? as u64),
            ))
        })?;

        // Rows arrive grouped by dictionary.
        let mut found: Vec<(DictionaryId, String, String, Vec<EntryLocator>)> = Vec::new();
        for row in rows {
            let (id, title, path, locator) = row?;
            match found.last_mut() {
                Some(last) if last.0 == id => last.3.push(locator),
                _ => found.push((id, title, path, vec![locator])),
            }
        }

        let sections = found
            .into_iter()
            .map(|(id, title, path, locators)| {
                let entries = self
                    .reader(id, Path::new(&path))
                    .and_then(|dict| locators.into_iter().map(|l| dict.entry(l)).collect())
                    .map_err(|e| e.to_string());
                ArticleSection {
                    dictionary_id: id,
                    dictionary_title: title,
                    entries,
                }
            })
            .collect();
        Ok(Article { sections })
    }

    /// A resource of dictionary `id` (stylesheet, image, font, sound, …), as
    /// articles reference it: `path` is relative to the dictionary, with `/`
    /// or `\` separators; leading separators are ignored.
    ///
    /// The dictionary's own resource files (MDict `.mdd` volumes, in volume
    /// order) are searched first, then loose files in the dictionary's
    /// folder. `None` means not found. That includes an unknown or unusable
    /// dictionary, a resource file that can't be read, junk paths, and paths
    /// that would leave the folder (`..`, absolute paths), which are refused.
    /// Dictionaries routinely reference resources the user's copy lacks, so
    /// none of this is an error. `Err` means the database failed.
    pub fn resource(&self, id: DictionaryId, path: &str) -> Result<Option<Resource>, LibraryError> {
        let Some(segments) = resource::segments(path) else {
            return Ok(None);
        };
        let dict_path: Option<String> = self
            .db
            .prepare_cached("SELECT path FROM dictionaries WHERE id = ?1 AND status = 'ready'")?
            .query_row([id.0], |row| row.get(0))
            .optional()?;
        let Some(dict_path) = dict_path.map(PathBuf::from) else {
            return Ok(None);
        };
        let relative = segments.join("/");
        let content_type = resource::content_type(&relative);

        // A dictionary or resource file that can't be read only means this
        // resource isn't there; the article itself reports unreadable files.
        let bundled = self
            .reader(id, &dict_path)
            .and_then(|dict| dict.resource(&relative))
            .ok()
            .flatten();
        let bytes = bundled.or_else(|| {
            let folder = dict_path.parent()?;
            fs::read(resource::loose_file(folder, &segments)?).ok()
        });
        Ok(bytes.map(|bytes| Resource { bytes, content_type }))
    }

    /// The open reader for dictionary `id`, opening `path` if needed.
    fn reader(&self, id: DictionaryId, path: &Path) -> Result<Arc<dyn Dictionary>, FormatError> {
        if let Some(dict) = self.lock_readers().get(&id) {
            return Ok(Arc::clone(dict));
        }
        // Open without holding the lock; a racing opener's copy is equal.
        let dict: Arc<dyn Dictionary> = Arc::from(format::open(path)?);
        Ok(Arc::clone(self.lock_readers().entry(id).or_insert(dict)))
    }

    fn lock_readers(&self) -> std::sync::MutexGuard<'_, HashMap<DictionaryId, Arc<dyn Dictionary>>> {
        // The map stays consistent even if a holder panicked.
        self.readers.lock().unwrap_or_else(PoisonError::into_inner)
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

/// SQL condition: headword row `h` belongs to a searchable dictionary.
///
/// The unary `+` stops SQLite from driving the query by the
/// `headwords_by_dictionary` index (a full scan plus sort); search must
/// walk the `folded` range of `headwords_by_folded` instead.
const SEARCHABLE: &str =
    "+h.dictionary_id IN (SELECT id FROM dictionaries WHERE enabled AND status = 'ready')";

fn to_sql_int(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// What the library has for one headword: see [`Library::article`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    /// One per dictionary that has the headword, in library order.
    pub sections: Vec<ArticleSection>,
}

/// One dictionary's part of an [`Article`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleSection {
    pub dictionary_id: DictionaryId,
    pub dictionary_title: String,
    /// The raw HTML of each entry, in file order, or why the dictionary
    /// could not be read (e.g. its file has gone).
    pub entries: Result<Vec<String>, String>,
}

/// A dictionary resource: see [`Library::resource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub bytes: Vec<u8>,
    /// MIME type, from the file extension (`application/octet-stream` if
    /// unknown).
    pub content_type: &'static str,
}

/// One page of [`Library::search`] results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    /// Headwords on this page, in result order.
    pub items: Vec<String>,
    /// Number of matching headwords across all pages.
    pub total_count: u64,
}

const SELECT_INFO: &str = "
    SELECT d.id, d.path, d.title, d.position, d.enabled, d.file_size, d.file_mtime_ns,
           d.status, d.error,
           (SELECT COUNT(*) FROM headwords h WHERE h.dictionary_id = d.id)
    FROM dictionaries d";

/// Identifies a dictionary within one library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DictionaryId(i64);

impl DictionaryId {
    /// The id for a number previously produced by [`DictionaryId::to_raw`]
    /// (e.g. received back from the frontend). An unknown number is not an
    /// error: lookups treat it as a dictionary that isn't there.
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// The number to hand to the frontend (and to `dict://` URLs).
    pub const fn to_raw(self) -> i64 {
        self.0
    }
}

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
