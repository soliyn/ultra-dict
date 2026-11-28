//! The Library: the one place that owns dictionaries, the headword index,
//! search, articles, resources and history (`docs/spec-v1.md`).
//!
//! Everything is persisted in one SQLite database. Callers (Tauri commands,
//! tests) use only this module's types; SQLite and the format layer stay
//! behind it.

mod fold;
mod indexer;
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

pub use indexer::{IndexEvent, Indexer, IndexerHandle, Progress};

/// Schema version written to `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 5;

/// `MIGRATIONS[n]` upgrades a schema-version `n + 1` database to `n + 2`.
/// A new database gets `schema.sql`, which is already the latest version.
const MIGRATIONS: &[&str] = &[
    // 1 → 2: per-dictionary "Allow scripts".
    "ALTER TABLE dictionaries ADD COLUMN allow_scripts INTEGER NOT NULL DEFAULT 0;",
    // 2 → 3: the user's own name for a dictionary.
    "ALTER TABLE dictionaries ADD COLUMN user_title TEXT;",
    // 3 → 4: remembered folders, for rescanning.
    "CREATE TABLE folders (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);",
    // 4 → 5: background indexing tells its runs apart.
    "ALTER TABLE dictionaries ADD COLUMN index_run INTEGER NOT NULL DEFAULT 0;",
];
const _: () = assert!(MIGRATIONS.len() as i64 == SCHEMA_VERSION - 1);

/// Dictionary readers opened so far, for articles and resources, shared
/// with the [`Indexer`]. Startup never opens them; each is opened on first
/// use.
type Readers = Arc<Mutex<HashMap<DictionaryId, Arc<dyn Dictionary>>>>;

/// The user's dictionary collection and its index.
///
/// Adding a dictionary only records it, with [`Status::Indexing`]. An
/// [`Indexer`] (see [`Library::indexer`]) reads its headwords on its own
/// database connection, so the library stays usable meanwhile.
pub struct Library {
    db: Connection,
    /// The database file, for the indexer's own connection.
    path: PathBuf,
    readers: Readers,
}

impl Library {
    /// Opens the library database at `path`, creating the file, its parent
    /// directories and the schema if they don't exist yet.
    pub fn open(path: &Path) -> Result<Self, LibraryError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut library = Self {
            db: connect(path)?,
            path: path.to_owned(),
            readers: Readers::default(),
        };
        library.migrate()?;
        library.recover_interrupted()?;
        Ok(library)
    }

    /// Indexing that was under way when the app last stopped never finished:
    /// those dictionaries need reindexing, and lose what was written.
    fn recover_interrupted(&mut self) -> Result<(), LibraryError> {
        let tx = self.db.transaction()?;
        tx.execute(
            "DELETE FROM headwords WHERE dictionary_id IN
                 (SELECT id FROM dictionaries WHERE status = 'indexing')",
            [],
        )?;
        tx.execute(
            "UPDATE dictionaries SET status = 'needs_reindex', index_run = index_run + 1
             WHERE status = 'indexing'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// An indexer for this library. It indexes the dictionaries waiting with
    /// [`Status::Indexing`], on its own connection to the database. Use one
    /// indexer per library, created after opening it.
    pub fn indexer(&self) -> Result<Indexer, LibraryError> {
        Ok(Indexer::new(connect(&self.path)?, Arc::clone(&self.readers)))
    }

    fn migrate(&mut self) -> Result<(), LibraryError> {
        let version: i64 = self.db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(LibraryError::NewerSchema(version));
        }
        if version == SCHEMA_VERSION {
            return Ok(());
        }
        let tx = self.db.transaction()?;
        if version == 0 {
            tx.execute_batch(include_str!("schema.sql"))?;
        } else {
            for step in &MIGRATIONS[(version - 1) as usize..] {
                tx.execute_batch(step)?;
            }
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the dictionary file at `path`, waiting to be indexed: it has
    /// [`Status::Indexing`], and the file name as its title until then.
    ///
    /// The file is referenced in place. Adding a file that is already in the
    /// library changes nothing and returns the existing record, however the
    /// path is spelled (`.`, `..`, links, letter case on Windows).
    ///
    /// A file the indexer can't use stays in the library with
    /// [`Status::Error`], so the user sees why. `Err` means the library
    /// itself (the database) failed.
    pub fn add_file(&mut self, path: &Path) -> Result<DictionaryInfo, LibraryError> {
        let path = normalize(path)?;
        Ok(self.add_normalized(&path)?.0)
    }

    /// Adds every dictionary file in the folder at `path` and its subfolders,
    /// as [`add_file`](Self::add_file) would (waiting to be indexed), in path
    /// order, and remembers
    /// the folder for [`rescan_folder`](Self::rescan_folder).
    ///
    /// Files already in the library are skipped, so adding a folder again (or
    /// one inside an added folder) only picks up what is new. Linked
    /// subfolders are not followed. A folder that can't be read is an `Err`,
    /// and nothing is recorded; unreadable subfolders are skipped.
    pub fn add_folder(&mut self, path: &Path) -> Result<FolderScan, LibraryError> {
        let path = dunce::canonicalize(path).map_err(|e| folder_error(path, e))?;
        let files = find_dictionary_files(&path)?;
        let path_text = path_text(&path)?;
        self.db.execute(
            "INSERT INTO folders (path) VALUES (?1) ON CONFLICT (path) DO NOTHING",
            [path_text],
        )?;
        let id = self
            .db
            .query_row("SELECT id FROM folders WHERE path = ?1", [path_text], |row| row.get(0))?;
        let folder = FolderInfo { id: FolderId(id), path };
        self.add_found(folder, files)
    }

    /// Scans a remembered folder again and adds the dictionary files placed
    /// there since (or removed from the library since: rescanning brings them
    /// back). Dictionaries already in the library are left as they are.
    ///
    /// A folder that can't be read any more is an `Err`; it stays remembered.
    pub fn rescan_folder(&mut self, id: FolderId) -> Result<FolderScan, LibraryError> {
        let folder = self
            .folders()?
            .into_iter()
            .find(|f| f.id == id)
            .ok_or(LibraryError::UnknownFolder(id))?;
        let files = find_dictionary_files(&folder.path)?;
        self.add_found(folder, files)
    }

    /// Every remembered folder, in the order added.
    pub fn folders(&self) -> Result<Vec<FolderInfo>, LibraryError> {
        let mut stmt = self.db.prepare_cached("SELECT id, path FROM folders ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(FolderInfo {
                id: FolderId(row.get(0)?),
                path: PathBuf::from(row.get::<_, String>(1)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Forgets folder `id`, so it is no longer offered for rescanning, and
    /// returns the remaining folders. The dictionaries found in it stay in
    /// the library.
    pub fn remove_folder(&mut self, id: FolderId) -> Result<Vec<FolderInfo>, LibraryError> {
        if self.db.execute("DELETE FROM folders WHERE id = ?1", [id.0])? == 0 {
            return Err(LibraryError::UnknownFolder(id));
        }
        self.folders()
    }

    /// Adds each of `files` (normalized) that isn't in the library yet.
    fn add_found(
        &mut self,
        folder: FolderInfo,
        files: Vec<PathBuf>,
    ) -> Result<FolderScan, LibraryError> {
        let mut added = Vec::new();
        for file in files {
            let (info, new) = self.add_normalized(&normalize(&file)?)?;
            if new {
                added.push(info);
            }
        }
        Ok(FolderScan { folder, added })
    }

    /// [`add_file`](Self::add_file) for a [`normalize`]d path. The flag says
    /// whether the dictionary is new (false: it was already in the library).
    fn add_normalized(&mut self, path: &Path) -> Result<(DictionaryInfo, bool), LibraryError> {
        let path_text = path_text(path)?;
        if let Some(existing) = self.find_by_path(path_text)? {
            return Ok((existing, false));
        }

        self.db.execute(
            "INSERT INTO dictionaries (path, title, position, enabled, status)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM dictionaries), 1,
                     'indexing')",
            params![path_text, format::display_title(None, path)],
        )?;
        let id = DictionaryId(self.db.last_insert_rowid());
        Ok((self.get(id)?, true))
    }

    /// Indexes dictionary `id` again from its file, whatever its status. It
    /// has [`Status::Indexing`] (not searchable) until an [`Indexer`] is
    /// done; its settings (name, order, enabled, scripts) are kept. A run
    /// already under way starts over.
    pub fn reindex(&mut self, id: DictionaryId) -> Result<DictionaryInfo, LibraryError> {
        self.db.execute(
            "UPDATE dictionaries SET status = 'indexing', error = NULL, index_run = index_run + 1
             WHERE id = ?1",
            [id.0],
        )?;
        self.get(id)
    }

    /// Stops indexing dictionary `id`, whether it is waiting or under way.
    /// It is left with [`Status::NeedsReindex`] and no headwords. A
    /// dictionary that isn't being indexed is left as it is.
    pub fn cancel_indexing(&mut self, id: DictionaryId) -> Result<DictionaryInfo, LibraryError> {
        let tx = self.db.transaction()?;
        let cancelled = tx.execute(
            "UPDATE dictionaries SET status = 'needs_reindex', index_run = index_run + 1
             WHERE id = ?1 AND status = 'indexing'",
            [id.0],
        )?;
        if cancelled > 0 {
            tx.execute("DELETE FROM headwords WHERE dictionary_id = ?1", [id.0])?;
        }
        tx.commit()?;
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
            "SELECT d.id, COALESCE(d.user_title, d.title), d.path, d.allow_scripts, h.locator
             FROM headwords h JOIN dictionaries d ON d.id = h.dictionary_id
             WHERE h.folded = ?1 AND h.headword = ?2 AND d.enabled AND d.status = 'ready'
             ORDER BY d.position, d.id, h.locator",
        )?;
        let rows = stmt.query_map(params![fold(headword), headword], |row| {
            Ok((
                DictionaryId(row.get(0)?),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                EntryLocator::from_raw(row.get::<_, i64>(4)? as u64),
            ))
        })?;

        /// One dictionary's matching entries, before they are read.
        struct Found {
            id: DictionaryId,
            title: String,
            path: String,
            allow_scripts: bool,
            locators: Vec<EntryLocator>,
        }

        // Rows arrive grouped by dictionary.
        let mut found: Vec<Found> = Vec::new();
        for row in rows {
            let (id, title, path, allow_scripts, locator) = row?;
            match found.last_mut() {
                Some(last) if last.id == id => last.locators.push(locator),
                _ => found.push(Found { id, title, path, allow_scripts, locators: vec![locator] }),
            }
        }

        let sections = found
            .into_iter()
            .map(|Found { id, title, path, allow_scripts, locators }| {
                let entries = self
                    .reader(id, Path::new(&path))
                    .and_then(|dict| locators.into_iter().map(|l| dict.entry(l)).collect())
                    .map_err(|e| e.to_string());
                ArticleSection {
                    dictionary_id: id,
                    dictionary_title: title,
                    allow_scripts,
                    entries,
                }
            })
            .collect();
        Ok(Article { sections })
    }

    /// Lets dictionary `id`'s own JavaScript run in its articles, or stops it.
    /// New dictionaries start with scripts off.
    pub fn set_allow_scripts(
        &mut self,
        id: DictionaryId,
        allow: bool,
    ) -> Result<DictionaryInfo, LibraryError> {
        self.update(id, "UPDATE dictionaries SET allow_scripts = ?2 WHERE id = ?1", allow)
    }

    /// Includes dictionary `id` in search and articles, or leaves it out
    /// while keeping it in the library. New dictionaries start enabled.
    pub fn set_enabled(
        &mut self,
        id: DictionaryId,
        enabled: bool,
    ) -> Result<DictionaryInfo, LibraryError> {
        self.update(id, "UPDATE dictionaries SET enabled = ?2 WHERE id = ?1", enabled)
    }

    /// Moves dictionary `id` to `position` in the library order (0 is first;
    /// past the end means last), shifting the others, and returns the new
    /// listing. The order drives the order of article sections.
    pub fn move_dictionary(
        &mut self,
        id: DictionaryId,
        position: u32,
    ) -> Result<Vec<DictionaryInfo>, LibraryError> {
        let tx = self.db.transaction()?;
        let mut order = library_order(&tx)?;
        let from = order
            .iter()
            .position(|&d| d == id)
            .ok_or(LibraryError::UnknownDictionary(id))?;
        order.remove(from);
        order.insert((position as usize).min(order.len()), id);
        write_order(&tx, &order)?;
        tx.commit()?;
        self.dictionaries()
    }

    /// Takes dictionary `id` out of the library, with its index and settings,
    /// and returns the new listing. The file itself is left alone; adding it
    /// again starts afresh.
    pub fn remove(&mut self, id: DictionaryId) -> Result<Vec<DictionaryInfo>, LibraryError> {
        let tx = self.db.transaction()?;
        // Headwords go with it (ON DELETE CASCADE).
        if tx.execute("DELETE FROM dictionaries WHERE id = ?1", [id.0])? == 0 {
            return Err(LibraryError::UnknownDictionary(id));
        }
        write_order(&tx, &library_order(&tx)?)?;
        tx.commit()?;
        self.lock_readers().remove(&id);
        self.dictionaries()
    }

    /// Gives dictionary `id` the display name `name` (trimmed), in place of
    /// the title from its file. An empty name restores the file's title.
    /// The name is the user's: reindexing the file doesn't change it.
    pub fn rename(&mut self, id: DictionaryId, name: &str) -> Result<DictionaryInfo, LibraryError> {
        let name = Some(name.trim()).filter(|n| !n.is_empty());
        self.update(id, "UPDATE dictionaries SET user_title = ?2 WHERE id = ?1", name)
    }

    /// Runs `sql` (an `UPDATE` of row `?1` setting `?2`) on dictionary `id`.
    fn update(
        &mut self,
        id: DictionaryId,
        sql: &str,
        value: impl rusqlite::ToSql,
    ) -> Result<DictionaryInfo, LibraryError> {
        let changed = self.db.prepare_cached(sql)?.execute(params![id.0, value])?;
        if changed == 0 {
            return Err(LibraryError::UnknownDictionary(id));
        }
        self.get(id)
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

    fn lock_readers(&self) -> ReadersGuard<'_> {
        lock_readers(&self.readers)
    }

    fn get(&self, id: DictionaryId) -> Result<DictionaryInfo, LibraryError> {
        let mut stmt = self.db.prepare_cached(&format!("{SELECT_INFO} WHERE d.id = ?1"))?;
        stmt.query_row([id.0], DictionaryInfo::from_row)
            .optional()?
            .ok_or(LibraryError::UnknownDictionary(id))
    }

    fn find_by_path(&self, path: &str) -> Result<Option<DictionaryInfo>, LibraryError> {
        let mut stmt = self
            .db
            .prepare_cached(&format!("{SELECT_INFO} WHERE d.path = ?1"))?;
        Ok(stmt.query_row([path], DictionaryInfo::from_row).optional()?)
    }

}

type ReadersGuard<'a> = std::sync::MutexGuard<'a, HashMap<DictionaryId, Arc<dyn Dictionary>>>;

fn lock_readers(readers: &Readers) -> ReadersGuard<'_> {
    // The map stays consistent even if a holder panicked.
    readers.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Opens a connection to the library database at `path`.
fn connect(path: &Path) -> Result<Connection, LibraryError> {
    let db = Connection::open(path)?;
    db.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    // The library and its indexer write through separate connections; each
    // waits out the other's short transactions.
    db.busy_timeout(Duration::from_secs(30))?;
    Ok(db)
}

/// The one spelling of `path` the library stores: canonical (`.` and `..`
/// resolved, links followed, the on-disk letter case on Windows) when the
/// file exists, so the same file is never added twice; otherwise just made
/// absolute.
fn normalize(path: &Path) -> Result<PathBuf, LibraryError> {
    match dunce::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(_) => Ok(std::path::absolute(path)?),
    }
}

fn path_text(path: &Path) -> Result<&str, LibraryError> {
    path.to_str().ok_or_else(|| LibraryError::PathNotUnicode(path.to_owned()))
}

fn folder_error(path: &Path, error: io::Error) -> LibraryError {
    LibraryError::Folder(path.to_owned(), error)
}

/// Every dictionary file under `folder`, recursively, sorted by path.
///
/// Only `.mdx` files (any letter case) for now; companion files such as
/// `.mdd` volumes are found from them. Linked subfolders are not followed,
/// so a link loop can't trap the scan. Subfolders that can't be read are
/// skipped; `folder` itself must be readable.
fn find_dictionary_files(folder: &Path) -> Result<Vec<PathBuf>, LibraryError> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_owned()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if dir == folder => return Err(folder_error(folder, error)),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if is_dictionary_file(&path) && path.is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn is_dictionary_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mdx"))
}

/// Every dictionary id, in library order.
fn library_order(tx: &Transaction<'_>) -> Result<Vec<DictionaryId>, LibraryError> {
    let mut stmt = tx.prepare_cached("SELECT id FROM dictionaries ORDER BY position, id")?;
    let ids = stmt.query_map([], |row| row.get(0).map(DictionaryId))?;
    Ok(ids.collect::<Result<_, _>>()?)
}

/// Numbers the dictionaries `0, 1, …` in the order of `ids`.
fn write_order(tx: &Transaction<'_>, ids: &[DictionaryId]) -> Result<(), LibraryError> {
    let mut stmt = tx.prepare_cached("UPDATE dictionaries SET position = ?2 WHERE id = ?1")?;
    for (position, id) in ids.iter().enumerate() {
        stmt.execute(params![id.0, to_sql_int(position)])?;
    }
    Ok(())
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
    /// Whether the dictionary's JavaScript may run: see
    /// [`Library::set_allow_scripts`].
    pub allow_scripts: bool,
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
    SELECT d.id, d.path, COALESCE(d.user_title, d.title), d.title, d.position, d.enabled,
           d.file_size, d.file_mtime_ns, d.status, d.error, d.allow_scripts,
           CASE WHEN d.status = 'ready'
                THEN (SELECT COUNT(*) FROM headwords h WHERE h.dictionary_id = d.id)
                ELSE 0 END
    FROM dictionaries d";

/// Identifies a dictionary within one library.
///
/// Once a dictionary is removed, a later one may get its id. Callers holding
/// ids (open articles, `dict://` URLs) refetch after every library change.
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

/// Identifies a remembered folder within one library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FolderId(i64);

impl FolderId {
    /// The id for a number previously produced by [`FolderId::to_raw`].
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// The number to hand to the frontend.
    pub const fn to_raw(self) -> i64 {
        self.0
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A folder the user added: see [`Library::add_folder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderInfo {
    pub id: FolderId,
    /// Canonical absolute path of the folder.
    pub path: PathBuf,
}

/// What adding or rescanning a folder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderScan {
    pub folder: FolderInfo,
    /// The dictionaries this scan added to the library, in path order,
    /// unusable ones (with [`Status::Error`]) included. Files that were
    /// already in the library are not listed.
    pub added: Vec<DictionaryInfo>,
}

/// A dictionary as the library lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryInfo {
    pub id: DictionaryId,
    /// Absolute path of the dictionary file.
    pub path: PathBuf,
    /// Display name: the user's rename, else [`file_title`](Self::file_title).
    pub title: String,
    /// The title read from the file (HTML stripped, falling back to the
    /// file name).
    pub file_title: String,
    /// Place in the library order (0 is first).
    pub position: u32,
    pub enabled: bool,
    /// Size and modification time of the file when it was last indexed, or
    /// `None` if it hasn't been yet or the file could not be read.
    pub file: Option<FileStamp>,
    pub status: Status,
    /// Whether the dictionary's own JavaScript may run in its articles
    /// (off for new dictionaries).
    pub allow_scripts: bool,
    /// Number of indexed headwords (0 unless [`Status::Ready`]).
    pub headword_count: u64,
}

impl DictionaryInfo {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let size: Option<i64> = row.get(6)?;
        let mtime_ns: Option<i64> = row.get(7)?;
        let status: String = row.get(8)?;
        let error: Option<String> = row.get(9)?;
        Ok(Self {
            id: DictionaryId(row.get(0)?),
            path: PathBuf::from(row.get::<_, String>(1)?),
            title: row.get(2)?,
            file_title: row.get(3)?,
            position: row.get(4)?,
            enabled: row.get(5)?,
            file: size.zip(mtime_ns).map(|(size, ns)| FileStamp {
                size: size as u64,
                modified: UNIX_EPOCH + Duration::from_nanos(ns as u64),
            }),
            status: match status.as_str() {
                "ready" => Status::Ready,
                "indexing" => Status::Indexing,
                "needs_reindex" => Status::NeedsReindex,
                _ => Status::Error(error.unwrap_or(status)),
            },
            allow_scripts: row.get(10)?,
            headword_count: row.get::<_, i64>(11)? as u64,
        })
    }
}

/// Where a dictionary stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Waiting to be indexed or being indexed, and not searchable yet. The
    /// [`Indexer`] reports progress.
    Indexing,
    /// Indexed and searchable.
    Ready,
    /// Indexing was cancelled or interrupted, and nothing of it was kept.
    /// See [`Library::reindex`].
    NeedsReindex,
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
    /// No dictionary in the library has this id.
    UnknownDictionary(DictionaryId),
    /// The library remembers no folder with this id.
    UnknownFolder(FolderId),
    /// A folder to add or rescan could not be read (missing, not a folder,
    /// no permission).
    Folder(PathBuf, io::Error),
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
            Self::UnknownDictionary(id) => write!(f, "no dictionary with id {id} in the library"),
            Self::UnknownFolder(id) => write!(f, "no folder with id {id} in the library"),
            Self::Folder(p, e) => write!(f, "cannot read folder {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for LibraryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::Io(e) | Self::Folder(_, e) => Some(e),
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
