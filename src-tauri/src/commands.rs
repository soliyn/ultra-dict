//! Tauri commands: thin pass-throughs to the [`Library`].
//!
//! Each command locks the shared library, makes one call, and converts the
//! result into a serializable shape for the frontend (camelCase fields).
//! Errors reach the frontend as readable strings: the rejected value of
//! `invoke`.
//!
//! The commands are `async` so that Tauri runs them off the main thread and
//! a long `add_file` does not freeze the window. Indexing still blocks other
//! commands until it finishes; background indexing is a later step.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::State;

use crate::library::{self, Library, LibraryError};

/// The library as Tauri state.
pub type SharedLibrary = Mutex<Library>;

/// Adds one dictionary file. A file that can't be used is still added, with
/// `status: "error"` and a message; only library failures reject.
#[tauri::command]
pub async fn add_file(
    library: State<'_, SharedLibrary>,
    path: PathBuf,
) -> Result<DictionaryInfo, String> {
    with_library(&library, |lib| lib.add_file(&path)).map(DictionaryInfo::from)
}

/// One page of headwords matching `query`.
#[tauri::command]
pub async fn search(
    library: State<'_, SharedLibrary>,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<SearchPage, String> {
    with_library(&library, |lib| lib.search(&query, offset, limit)).map(SearchPage::from)
}

/// Every entry for exactly `headword`, grouped by dictionary.
#[tauri::command]
pub async fn article(
    library: State<'_, SharedLibrary>,
    headword: String,
) -> Result<Article, String> {
    with_library(&library, |lib| lib.article(&headword)).map(Article::from)
}

fn with_library<T>(
    library: &SharedLibrary,
    call: impl FnOnce(&mut Library) -> Result<T, LibraryError>,
) -> Result<T, String> {
    // A panic mid-call can't leave the database inconsistent (writes are
    // transactional), so keep serving after one.
    let mut library = library.lock().unwrap_or_else(PoisonError::into_inner);
    call(&mut library).map_err(|e| e.to_string())
}

/// A dictionary in the library.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryInfo {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub position: u32,
    pub enabled: bool,
    /// `"ready"` or `"error"`.
    pub status: &'static str,
    /// Why the dictionary can't be used, when `status` is `"error"`.
    pub error: Option<String>,
    pub headword_count: u64,
}

impl From<library::DictionaryInfo> for DictionaryInfo {
    fn from(info: library::DictionaryInfo) -> Self {
        let (status, error) = match info.status {
            library::Status::Ready => ("ready", None),
            library::Status::Error(message) => ("error", Some(message)),
        };
        Self {
            id: info.id.to_raw(),
            path: info.path.to_string_lossy().into_owned(),
            title: info.title,
            position: info.position,
            enabled: info.enabled,
            status,
            error,
            headword_count: info.headword_count,
        }
    }
}

/// One page of search results.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<String>,
    pub total_count: u64,
}

impl From<library::SearchPage> for SearchPage {
    fn from(page: library::SearchPage) -> Self {
        Self {
            items: page.items,
            total_count: page.total_count,
        }
    }
}

/// What the library has for one headword.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub sections: Vec<ArticleSection>,
}

/// One dictionary's part of an [`Article`]: either `entries` (raw HTML, in
/// file order) or `error`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleSection {
    pub dictionary_id: i64,
    pub dictionary_title: String,
    pub entries: Vec<String>,
    pub error: Option<String>,
}

impl From<library::Article> for Article {
    fn from(article: library::Article) -> Self {
        let sections = article
            .sections
            .into_iter()
            .map(|section| {
                let (entries, error) = match section.entries {
                    Ok(entries) => (entries, None),
                    Err(message) => (Vec::new(), Some(message)),
                };
                ArticleSection {
                    dictionary_id: section.dictionary_id.to_raw(),
                    dictionary_title: section.dictionary_title,
                    entries,
                    error,
                }
            })
            .collect();
        Self { sections }
    }
}
