// The single main window: search box and results on one side, the article
// area on the other.

import { KeyboardEvent, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as api from "./api";
import { ArticleView } from "./ArticleView";
import { ResultsList } from "./ResultsList";
import { strings } from "./strings";
import { useSearch } from "./useSearch";
import "./App.css";

const RESULTS_ID = "results";

export default function App() {
  const [query, setQuery] = useState("");
  // Bumped after each add, so results and the open article are fetched again.
  const [libraryVersion, setLibraryVersion] = useState(0);
  const results = useSearch(query, libraryVersion);
  const resultKey = `${libraryVersion}:${query}`;
  const searchRef = useRef<HTMLInputElement>(null);

  // The highlighted row of the current results, and the headword it holds.
  // The headword outlives the row: typing a new query keeps it shown.
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [selectedHeadword, setSelectedHeadword] = useState<string | null>(null);
  const [selectionKey, setSelectionKey] = useState(resultKey);
  if (selectionKey !== resultKey) {
    setSelectionKey(resultKey);
    setSelectedIndex(null);
  }

  const [adding, setAdding] = useState(false);
  // A failed add, shown until dismissed or the next add.
  const [notice, setNotice] = useState<string | null>(null);

  const addFile = async () => {
    setAdding(true);
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: strings.dictionaryFilter, extensions: ["mdx"] }],
      });
      searchRef.current?.focus();
      if (path === null) return;
      setNotice(null);
      const info = await api.addFile(path);
      if (info.status === "error") {
        setNotice(strings.dictionaryUnusable(info.title, info.error ?? ""));
      } else {
        setLibraryVersion((v) => v + 1);
      }
    } catch (e) {
      setNotice(strings.addFailed(String(e)));
    } finally {
      setAdding(false);
    }
  };

  // The row's headword may still be loading (arrow keys across a page edge).
  const { item } = results;
  useEffect(() => {
    if (selectedIndex === null) return;
    const headword = item(selectedIndex);
    if (headword !== undefined) setSelectedHeadword(headword);
  }, [item, selectedIndex]);

  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      // Open the best match (row 0: exact case first), unless a row is
      // already open. The row may still be loading; the effect above waits.
      e.preventDefault();
      if (results.totalCount !== 0) setSelectedIndex((i) => i ?? 0);
      return;
    }
    const count = results.totalCount ?? 0;
    if (count === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => (i === null ? 0 : Math.min(i + 1, count - 1)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => (i === null ? 0 : Math.max(i - 1, 0)));
    }
  };

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="toolbar">
          <input
            ref={searchRef}
            className="search"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onSearchKeyDown}
            placeholder={strings.searchPlaceholder}
            aria-label={strings.searchLabel}
            aria-controls={RESULTS_ID}
            autoFocus
            spellCheck={false}
            autoComplete="off"
          />
          <button className="add-file" onClick={addFile} disabled={adding}>
            {adding ? strings.adding : strings.addFile}
          </button>
        </div>
        {notice !== null && (
          <div className="notice" role="alert">
            <span>{notice}</span>
            <button className="notice-dismiss" onClick={() => setNotice(null)} aria-label={strings.dismiss}>
              ×
            </button>
          </div>
        )}
        <ResultsList
          id={RESULTS_ID}
          results={results}
          query={query}
          resultKey={resultKey}
          selectedIndex={selectedIndex}
          onSelect={setSelectedIndex}
        />
        <div className="status">
          {results.totalCount ? strings.matchCount(results.totalCount) : " "}
        </div>
      </aside>
      <main className="article-pane">
        {selectedHeadword === null ? (
          <p className="placeholder">{strings.noSelection}</p>
        ) : (
          <ArticleView headword={selectedHeadword} libraryVersion={libraryVersion} />
        )}
      </main>
    </div>
  );
}
