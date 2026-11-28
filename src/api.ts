// Typed wrappers for the Rust commands in src-tauri/src/commands.rs.
// The interfaces mirror the Rust DTOs there: change both together.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export interface DictionaryInfo {
  id: number;
  path: string;
  /** Display name: the user's rename, else `fileTitle`. */
  title: string;
  /** The title read from the file. */
  fileTitle: string;
  position: number;
  enabled: boolean;
  /**
   * `"indexing"`: waiting for or being indexed in the background, not
   * searchable yet (progress comes as `IndexingEvent`s). `"needs_reindex"`:
   * indexing was stopped or interrupted. `"missing"`: the file was gone at
   * startup (see `relocateDictionary`).
   */
  status: "indexing" | "ready" | "needs_reindex" | "missing" | "error";
  /** Why the dictionary can't be used, when `status` is `"error"`. */
  error: string | null;
  /** Whether the dictionary's own JavaScript may run in its articles. */
  allowScripts: boolean;
  headwordCount: number;
}

/**
 * How far the background indexer has got with a dictionary, or (`finished`)
 * that it is done with it, whatever the outcome: refetch to see which.
 */
export interface IndexingEvent {
  dictionaryId: number;
  finished: boolean;
  /** Headwords written so far (0 when finished). */
  done: number;
  /** Headwords expected (0 when finished). */
  total: number;
}

/** Calls `handler` for each indexer event. Resolves to the unsubscribe function. */
export function onIndexing(handler: (event: IndexingEvent) => void): Promise<UnlistenFn> {
  return listen<IndexingEvent>("indexing", (e) => handler(e.payload));
}

export interface FolderInfo {
  id: number;
  path: string;
}

export interface FolderScan {
  folder: FolderInfo;
  /** Only the dictionaries new to the library, unusable ones included. */
  added: DictionaryInfo[];
}

export interface SearchPage {
  items: string[];
  totalCount: number;
}

export interface ArticleSection {
  dictionaryId: number;
  dictionaryTitle: string;
  /** Whether the dictionary's JavaScript may run in its frame. */
  allowScripts: boolean;
  /**
   * The headword whose entries are shown, when the one looked up redirects
   * there (`went` → `go`); null when they are its own.
   */
  redirect: string | null;
  /** Raw entry HTML, in file order. Empty when `error` is set. */
  entries: string[];
  error: string | null;
}

export interface Article {
  sections: ArticleSection[];
}

// Each call rejects with a readable message string when the library fails.

/** Resolves at once; the dictionary is indexed in the background. */
export function addFile(path: string): Promise<DictionaryInfo> {
  return invoke("add_file", { path });
}

/** Adds every dictionary in the folder (recursively) and remembers the folder. */
export function addFolder(path: string): Promise<FolderScan> {
  return invoke("add_folder", { path });
}

/** Adds the dictionaries placed in a remembered folder since. */
export function rescanFolder(folderId: number): Promise<FolderScan> {
  return invoke("rescan_folder", { folderId });
}

/** Every remembered folder, in the order added. */
export function folders(): Promise<FolderInfo[]> {
  return invoke("folders");
}

/** Resolves to the remaining folders. The folder's dictionaries stay. */
export function removeFolder(folderId: number): Promise<FolderInfo[]> {
  return invoke("remove_folder", { folderId });
}

export function search(query: string, offset: number, limit: number): Promise<SearchPage> {
  return invoke("search", { query, offset, limit });
}

export function article(headword: string): Promise<Article> {
  return invoke("article", { headword });
}

export function setAllowScripts(dictionaryId: number, allow: boolean): Promise<DictionaryInfo> {
  return invoke("set_allow_scripts", { dictionaryId, allow });
}

/** Every dictionary, in library order. */
export function dictionaries(): Promise<DictionaryInfo[]> {
  return invoke("dictionaries");
}

export function setEnabled(dictionaryId: number, enabled: boolean): Promise<DictionaryInfo> {
  return invoke("set_enabled", { dictionaryId, enabled });
}

/** An empty `name` restores the file's title. */
export function renameDictionary(dictionaryId: number, name: string): Promise<DictionaryInfo> {
  return invoke("rename_dictionary", { dictionaryId, name });
}

/** Resolves to the new listing. */
export function moveDictionary(dictionaryId: number, position: number): Promise<DictionaryInfo[]> {
  return invoke("move_dictionary", { dictionaryId, position });
}

/** Indexes the dictionary again from its file, in the background. */
export function reindexDictionary(dictionaryId: number): Promise<DictionaryInfo> {
  return invoke("reindex_dictionary", { dictionaryId });
}

/**
 * Points the dictionary at another file and indexes it from there in the
 * background, keeping its name, order and settings. Rejects if that file is
 * already in the library.
 */
export function relocateDictionary(dictionaryId: number, path: string): Promise<DictionaryInfo> {
  return invoke("relocate_dictionary", { dictionaryId, path });
}

/** Stops indexing the dictionary; it is left needing reindexing. */
export function cancelIndexing(dictionaryId: number): Promise<DictionaryInfo> {
  return invoke("cancel_indexing", { dictionaryId });
}

/** Resolves to the new listing. The file itself is left alone. */
export function removeDictionary(dictionaryId: number): Promise<DictionaryInfo[]> {
  return invoke("remove_dictionary", { dictionaryId });
}
