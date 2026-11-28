//! Tauri commands: thin pass-throughs to the [`Library`].
//!
//! Each command locks the shared library, makes one call, and converts the
//! result into a serializable shape for the frontend (camelCase fields).
//! Errors reach the frontend as readable strings: the rejected value of
//! `invoke`.
//!
//! The commands are `async` so that Tauri runs them off the main thread and
//! a long `add_file` or `add_folder` does not freeze the window. Indexing
//! still blocks other commands until it finishes; background indexing is a
//! later step.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::library::{self, DictionaryId, FolderId, Library, LibraryError};
use crate::{SharedLibrary, lock_library};

/// Adds one dictionary file. A file that can't be used is still added, with
/// `status: "error"` and a message; only library failures reject.
#[tauri::command]
pub async fn add_file(
    library: State<'_, SharedLibrary>,
    path: PathBuf,
) -> Result<DictionaryInfo, String> {
    with_library(&library, |lib| lib.add_file(&path)).map(DictionaryInfo::from)
}

/// Adds every dictionary in a folder (recursively) and remembers the folder.
/// Rejects if the folder can't be read.
#[tauri::command]
pub async fn add_folder(
    library: State<'_, SharedLibrary>,
    path: PathBuf,
) -> Result<FolderScan, String> {
    with_library(&library, |lib| lib.add_folder(&path)).map(FolderScan::from)
}

/// Adds the dictionaries placed in a remembered folder since it was scanned.
#[tauri::command]
pub async fn rescan_folder(
    library: State<'_, SharedLibrary>,
    folder_id: i64,
) -> Result<FolderScan, String> {
    let id = FolderId::from_raw(folder_id);
    with_library(&library, |lib| lib.rescan_folder(id)).map(FolderScan::from)
}

/// Every remembered folder.
#[tauri::command]
pub async fn folders(library: State<'_, SharedLibrary>) -> Result<Vec<FolderInfo>, String> {
    with_library(&library, |lib| lib.folders()).map(folder_list)
}

/// Forgets a folder (its dictionaries stay); returns the remaining folders.
#[tauri::command]
pub async fn remove_folder(
    library: State<'_, SharedLibrary>,
    folder_id: i64,
) -> Result<Vec<FolderInfo>, String> {
    let id = FolderId::from_raw(folder_id);
    with_library(&library, |lib| lib.remove_folder(id)).map(folder_list)
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

/// Lets a dictionary's own JavaScript run in its articles, or stops it.
#[tauri::command]
pub async fn set_allow_scripts(
    library: State<'_, SharedLibrary>,
    dictionary_id: i64,
    allow: bool,
) -> Result<DictionaryInfo, String> {
    let id = DictionaryId::from_raw(dictionary_id);
    with_library(&library, |lib| lib.set_allow_scripts(id, allow)).map(DictionaryInfo::from)
}

/// Every dictionary, in library order.
#[tauri::command]
pub async fn dictionaries(library: State<'_, SharedLibrary>) -> Result<Vec<DictionaryInfo>, String> {
    with_library(&library, |lib| lib.dictionaries()).map(list)
}

/// Includes a dictionary in search and articles, or leaves it out.
#[tauri::command]
pub async fn set_enabled(
    library: State<'_, SharedLibrary>,
    dictionary_id: i64,
    enabled: bool,
) -> Result<DictionaryInfo, String> {
    let id = DictionaryId::from_raw(dictionary_id);
    with_library(&library, |lib| lib.set_enabled(id, enabled)).map(DictionaryInfo::from)
}

/// Gives a dictionary the user's own name; an empty name restores the
/// file's title.
#[tauri::command]
pub async fn rename_dictionary(
    library: State<'_, SharedLibrary>,
    dictionary_id: i64,
    name: String,
) -> Result<DictionaryInfo, String> {
    let id = DictionaryId::from_raw(dictionary_id);
    with_library(&library, |lib| lib.rename(id, &name)).map(DictionaryInfo::from)
}

/// Moves a dictionary to `position` in the library order; returns the new
/// listing.
#[tauri::command]
pub async fn move_dictionary(
    library: State<'_, SharedLibrary>,
    dictionary_id: i64,
    position: u32,
) -> Result<Vec<DictionaryInfo>, String> {
    let id = DictionaryId::from_raw(dictionary_id);
    with_library(&library, |lib| lib.move_dictionary(id, position)).map(list)
}

/// Takes a dictionary out of the library (the file stays); returns the new
/// listing.
#[tauri::command]
pub async fn remove_dictionary(
    library: State<'_, SharedLibrary>,
    dictionary_id: i64,
) -> Result<Vec<DictionaryInfo>, String> {
    let id = DictionaryId::from_raw(dictionary_id);
    with_library(&library, |lib| lib.remove(id)).map(list)
}

fn list(infos: Vec<library::DictionaryInfo>) -> Vec<DictionaryInfo> {
    infos.into_iter().map(DictionaryInfo::from).collect()
}

fn folder_list(folders: Vec<library::FolderInfo>) -> Vec<FolderInfo> {
    folders.into_iter().map(FolderInfo::from).collect()
}

fn with_library<T>(
    library: &SharedLibrary,
    call: impl FnOnce(&mut Library) -> Result<T, LibraryError>,
) -> Result<T, String> {
    let mut library = lock_library(library);
    call(&mut library).map_err(|e| e.to_string())
}

/// A dictionary in the library.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryInfo {
    pub id: i64,
    pub path: String,
    /// Display name: the user's rename, else `file_title`.
    pub title: String,
    /// The title read from the file.
    pub file_title: String,
    pub position: u32,
    pub enabled: bool,
    /// `"ready"` or `"error"`.
    pub status: &'static str,
    /// Why the dictionary can't be used, when `status` is `"error"`.
    pub error: Option<String>,
    pub allow_scripts: bool,
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
            file_title: info.file_title,
            position: info.position,
            enabled: info.enabled,
            status,
            error,
            allow_scripts: info.allow_scripts,
            headword_count: info.headword_count,
        }
    }
}

/// A folder the user added.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfo {
    pub id: i64,
    pub path: String,
}

impl From<library::FolderInfo> for FolderInfo {
    fn from(folder: library::FolderInfo) -> Self {
        Self {
            id: folder.id.to_raw(),
            path: folder.path.to_string_lossy().into_owned(),
        }
    }
}

/// What adding or rescanning a folder did: `added` lists only the
/// dictionaries new to the library.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderScan {
    pub folder: FolderInfo,
    pub added: Vec<DictionaryInfo>,
}

impl From<library::FolderScan> for FolderScan {
    fn from(scan: library::FolderScan) -> Self {
        Self {
            folder: scan.folder.into(),
            added: list(scan.added),
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
    /// Whether the dictionary's JavaScript may run in its frame.
    pub allow_scripts: bool,
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
                    allow_scripts: section.allow_scripts,
                    entries,
                    error,
                }
            })
            .collect();
        Self { sections }
    }
}
