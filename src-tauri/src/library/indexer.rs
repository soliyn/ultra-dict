//! Indexing: reading a dictionary's headwords into the database, off the
//! library's own connection so that search and articles keep working.
//!
//! The database is the work queue: every dictionary with status `indexing`
//! is waiting for the [`Indexer`]. It writes headwords in short batches, and
//! before each one checks that the dictionary still waits for this very run
//! (`index_run`). That is how [`Library::cancel_indexing`],
//! [`Library::reindex`] and [`Library::remove`] stop a run under way: they
//! change the row, and the indexer stops at its next batch. A dictionary
//! becomes searchable only in the transaction that sets it `ready`.
//!
//! [`Library::cancel_indexing`]: super::Library::cancel_indexing
//! [`Library::reindex`]: super::Library::reindex
//! [`Library::remove`]: super::Library::remove

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::{DictionaryId, FileStamp, LibraryError, Readers, fold, lock_readers};
use crate::format;

/// Headwords written per transaction: small enough that cancelling, and
/// the library's own writes, wait well under a second.
const DEFAULT_BATCH: usize = 5_000;

/// Indexes the dictionaries a [`Library`](super::Library) has waiting.
/// Get one with [`Library::indexer`](super::Library::indexer).
pub struct Indexer {
    db: Connection,
    readers: Readers,
    batch: usize,
}

/// How far indexing one dictionary has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Headwords written so far.
    pub done: u64,
    /// Headwords the dictionary declares (may be an estimate).
    pub total: u64,
}

/// What a background indexer ([`Indexer::spawn`]) reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexEvent {
    /// Indexing `dictionary` has got this far. The first event of a run has
    /// `done == 0`.
    Progress { dictionary: DictionaryId, progress: Progress },
    /// The indexer is done with `dictionary`, whatever the outcome (ready,
    /// error, cancelled or removed). The library has the new state.
    Finished { dictionary: DictionaryId },
}

/// Keeps a background indexer running. Dropping it stops the indexer at its
/// next batch, leaving a run under way to be recovered as interrupted.
pub struct IndexerHandle {
    wake: Sender<()>,
}

impl IndexerHandle {
    /// Tells the indexer that the library has new work waiting.
    pub fn wake(&self) {
        // A stopped indexer has nothing to wake.
        let _ = self.wake.send(());
    }
}

/// Where one indexing run stands, as the database sees it.
enum Run {
    Continue,
    /// Cancelled, restarted or removed: someone else owns the row now.
    Superseded,
}

impl Indexer {
    pub(super) fn new(db: Connection, readers: Readers) -> Self {
        Self { db, readers, batch: DEFAULT_BATCH }
    }

    /// Writes `headwords` headwords per transaction (at least 1) instead of
    /// the default 5,000. Smaller batches report progress and notice
    /// cancelling sooner, and index more slowly.
    pub fn batch_size(mut self, headwords: usize) -> Self {
        self.batch = headwords.max(1);
        self
    }

    /// Indexes the next waiting dictionary to the end, and returns its id,
    /// or `None` if none is waiting.
    ///
    /// `on_progress` is called after each batch of headwords is written.
    /// Returning `Break` stops at once and leaves the run as an interrupted
    /// one: the dictionary stays [`Status::Indexing`](super::Status) until
    /// the library is next opened, which marks it as needing reindexing.
    ///
    /// A dictionary that can't be opened or read ends with
    /// [`Status::Error`](super::Status); that is not an `Err`. `Err` means
    /// the database failed.
    pub fn index_next(
        &mut self,
        mut on_progress: impl FnMut(DictionaryId, Progress) -> ControlFlow<()>,
    ) -> Result<Option<DictionaryId>, LibraryError> {
        let next: Option<(i64, String, i64)> = self
            .db
            .query_row(
                "SELECT id, path, index_run FROM dictionaries WHERE status = 'indexing'
                 ORDER BY id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((id, path, run)) = next else {
            return Ok(None);
        };
        let id = DictionaryId(id);
        self.index(id, Path::new(&path), run, &mut on_progress)?;
        Ok(Some(id))
    }

    fn index(
        &mut self,
        id: DictionaryId,
        path: &Path,
        run: i64,
        on_progress: &mut dyn FnMut(DictionaryId, Progress) -> ControlFlow<()>,
    ) -> Result<(), LibraryError> {
        let stamp = FileStamp::read(path);
        let dict = match format::open(path) {
            Ok(dict) => dict,
            Err(error) => {
                let (tx, state) = self.begin(id, run)?;
                if let Run::Continue = state {
                    finish(&tx, id, None, stamp, Err(&error.to_string()))?;
                    tx.commit()?;
                }
                return Ok(());
            }
        };
        let title = dict.metadata().title.clone();
        let total = dict.headword_count();

        // A run that started over may have left headwords behind.
        let (tx, state) = self.begin(id, run)?;
        if let Run::Superseded = state {
            return Ok(());
        }
        tx.execute("DELETE FROM headwords WHERE dictionary_id = ?1", [id.0])?;
        tx.commit()?;

        let batch = self.batch;
        let mut done = 0u64;
        if on_progress(id, Progress { done, total }).is_break() {
            return Ok(());
        }
        let mut headwords = dict.headwords();
        loop {
            let (tx, state) = self.begin(id, run)?;
            if let Run::Superseded = state {
                return Ok(());
            }
            let mut ended = false;
            let mut failure = None;
            {
                let mut insert = tx.prepare_cached(
                    "INSERT INTO headwords (dictionary_id, headword, folded, locator)
                     VALUES (?1, ?2, ?3, ?4)",
                )?;
                for _ in 0..batch {
                    match headwords.next() {
                        Some(Ok(headword)) => {
                            insert.execute(params![
                                id.0,
                                headword.text,
                                fold(&headword.text),
                                // Bit-preserving: locators round-trip through i64.
                                headword.locator.to_raw() as i64,
                            ])?;
                            done += 1;
                        }
                        Some(Err(error)) => {
                            failure = Some(error.to_string());
                            break;
                        }
                        None => {
                            ended = true;
                            break;
                        }
                    }
                }
            }
            if let Some(message) = failure {
                // Nothing of a failed run is kept.
                tx.execute("DELETE FROM headwords WHERE dictionary_id = ?1", [id.0])?;
                finish(&tx, id, Some(&title), stamp, Err(&message))?;
                tx.commit()?;
                return Ok(());
            }
            if ended {
                finish(&tx, id, Some(&title), stamp, Ok(()))?;
                tx.commit()?;
                drop(headwords);
                // A reader opened before this run may be of an older file;
                // this one is current, and articles will want it.
                lock_readers(&self.readers).insert(id, Arc::from(dict));
                return Ok(());
            }
            tx.commit()?;
            if on_progress(id, Progress { done, total }).is_break() {
                return Ok(());
            }
        }
    }

    /// Starts a write transaction, and says whether dictionary `id` still
    /// waits for run `run`.
    fn begin(&mut self, id: DictionaryId, run: i64) -> Result<(Transaction<'_>, Run), LibraryError> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ours = tx
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM dictionaries
                 WHERE id = ?1 AND status = 'indexing' AND index_run = ?2)",
            )?
            .query_row(params![id.0, run], |row| row.get::<_, bool>(0))?;
        Ok((tx, if ours { Run::Continue } else { Run::Superseded }))
    }

    /// Runs the indexer on a thread of its own until the returned handle is
    /// dropped, reporting to `on_event` (called on that thread).
    ///
    /// It indexes whatever is waiting, then sleeps until
    /// [`IndexerHandle::wake`].
    pub fn spawn(mut self, mut on_event: impl FnMut(IndexEvent) + Send + 'static) -> IndexerHandle {
        let (wake, woken) = mpsc::channel::<()>();
        thread::Builder::new()
            .name("indexer".to_owned())
            .spawn(move || {
                loop {
                    let mut stopped = false;
                    let indexed = self.index_next(|dictionary, progress| {
                        // Wake-ups mid-run need nothing: the queue is
                        // drained before sleeping.
                        if let Err(TryRecvError::Disconnected) = woken.try_recv() {
                            stopped = true;
                            return ControlFlow::Break(());
                        }
                        on_event(IndexEvent::Progress { dictionary, progress });
                        ControlFlow::Continue(())
                    });
                    match indexed {
                        _ if stopped => return,
                        Ok(Some(dictionary)) => on_event(IndexEvent::Finished { dictionary }),
                        Ok(None) => {
                            if woken.recv().is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            // The dictionary stays waiting; try again when
                            // there is new work.
                            eprintln!("indexing failed: {error}");
                            if woken.recv().is_err() {
                                return;
                            }
                        }
                    }
                }
            })
            .expect("start the indexer thread");
        IndexerHandle { wake }
    }
}

/// Ends run of dictionary `id`: `ready` with its file's title, or `error`.
fn finish(
    tx: &Transaction<'_>,
    id: DictionaryId,
    title: Option<&str>,
    stamp: Option<FileStamp>,
    outcome: Result<(), &str>,
) -> Result<(), LibraryError> {
    let (status, error) = match outcome {
        Ok(()) => ("ready", None),
        Err(message) => ("error", Some(message)),
    };
    tx.execute(
        "UPDATE dictionaries
         SET status = ?2, error = ?3, title = COALESCE(?4, title), file_size = ?5, file_mtime_ns = ?6
         WHERE id = ?1",
        params![
            id.0,
            status,
            error,
            title,
            stamp.map(|s| s.size as i64),
            stamp.map(|s| s.modified_ns()),
        ],
    )?;
    Ok(())
}
