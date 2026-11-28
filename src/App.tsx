// The single main window: search box and results on one side, the article
// area (or the dictionary list) on the other.

import { KeyboardEvent, useCallback, useEffect, useRef, useState } from "react";
import * as api from "./api";
import { ArticleView } from "./ArticleView";
import { LibraryView } from "./LibraryView";
import { ResultsList } from "./ResultsList";
import { strings } from "./strings";
import { useSearch } from "./useSearch";
import "./App.css";

const RESULTS_ID = "results";

export default function App() {
  const [query, setQuery] = useState("");
  // Bumped after each library change, so results, the open article and the
  // dictionary list are fetched again.
  const [libraryVersion, setLibraryVersion] = useState(0);
  const libraryChanged = useCallback(() => setLibraryVersion((v) => v + 1), []);
  // A dictionary the indexer is done with may have become searchable.
  useEffect(() => {
    const unlisten = api.onIndexing((event) => event.finished && libraryChanged());
    return () => void unlisten.then((stop) => stop());
  }, [libraryChanged]);
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

  // The dictionary list replaces the article while open. It opens by itself
  // on an empty library, where adding a dictionary is the only thing to do.
  const [showLibrary, setShowLibrary] = useState(false);
  useEffect(() => {
    api
      .dictionaries()
      .then((list) => list.length === 0 && setShowLibrary(true))
      .catch(() => {}); // the search reports a broken library
  }, []);
  const closeLibrary = () => {
    setShowLibrary(false);
    searchRef.current?.focus();
  };

  const select = (index: number | ((i: number | null) => number)) => {
    setSelectedIndex(index);
    setShowLibrary(false);
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
      if (results.totalCount !== 0) select((i) => i ?? 0);
      return;
    }
    const count = results.totalCount ?? 0;
    if (count === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      select((i) => (i === null ? 0 : Math.min(i + 1, count - 1)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      select((i) => (i === null ? 0 : Math.max(i - 1, 0)));
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
          <button
            className={"button" + (showLibrary ? " pressed" : "")}
            onClick={() => (showLibrary ? closeLibrary() : setShowLibrary(true))}
            aria-pressed={showLibrary}
          >
            {strings.dictionaries}
          </button>
        </div>
        <ResultsList
          id={RESULTS_ID}
          results={results}
          query={query}
          resultKey={resultKey}
          selectedIndex={selectedIndex}
          onSelect={select}
        />
        <div className="status">
          {results.totalCount ? strings.matchCount(results.totalCount) : " "}
        </div>
      </aside>
      <main className="article-pane">
        {showLibrary ? (
          <LibraryView libraryVersion={libraryVersion} onChanged={libraryChanged} onClose={closeLibrary} />
        ) : selectedHeadword === null ? (
          <p className="placeholder">{strings.noSelection}</p>
        ) : (
          <ArticleView headword={selectedHeadword} libraryVersion={libraryVersion} />
        )}
      </main>
    </div>
  );
}
