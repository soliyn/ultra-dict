// Typed wrappers for the Rust commands in src-tauri/src/commands.rs.
// The interfaces mirror the Rust DTOs there: change both together.

import { invoke } from "@tauri-apps/api/core";

export interface DictionaryInfo {
  id: number;
  path: string;
  title: string;
  position: number;
  enabled: boolean;
  status: "ready" | "error";
  /** Why the dictionary can't be used, when `status` is `"error"`. */
  error: string | null;
  headwordCount: number;
}

export interface SearchPage {
  items: string[];
  totalCount: number;
}

export interface ArticleSection {
  dictionaryId: number;
  dictionaryTitle: string;
  /** Raw entry HTML, in file order. Empty when `error` is set. */
  entries: string[];
  error: string | null;
}

export interface Article {
  sections: ArticleSection[];
}

// Each call rejects with a readable message string when the library fails.

export function addFile(path: string): Promise<DictionaryInfo> {
  return invoke("add_file", { path });
}

export function search(query: string, offset: number, limit: number): Promise<SearchPage> {
  return invoke("search", { query, offset, limit });
}

export function article(headword: string): Promise<Article> {
  return invoke("article", { headword });
}
