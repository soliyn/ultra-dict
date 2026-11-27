// The single main window: search box and results on one side, the article
// area on the other.

import { KeyboardEvent, useEffect, useState } from "react";
import { ResultsList } from "./ResultsList";
import { strings } from "./strings";
import { useSearch } from "./useSearch";
import "./App.css";

const RESULTS_ID = "results";

export default function App() {
  const [query, setQuery] = useState("");
  const results = useSearch(query);

  // The highlighted row of the current results, and the headword it holds.
  // The headword outlives the row: typing a new query keeps it shown.
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [selectedHeadword, setSelectedHeadword] = useState<string | null>(null);
  const [selectionQuery, setSelectionQuery] = useState(query);
  if (selectionQuery !== query) {
    setSelectionQuery(query);
    setSelectedIndex(null);
  }

  // The row's headword may still be loading (arrow keys across a page edge).
  const { item } = results;
  useEffect(() => {
    if (selectedIndex === null) return;
    const headword = item(selectedIndex);
    if (headword !== undefined) setSelectedHeadword(headword);
  }, [item, selectedIndex]);

  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
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
        <input
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
        <ResultsList
          id={RESULTS_ID}
          results={results}
          resultKey={query}
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
          <h1 className="article-headword">{selectedHeadword}</h1>
        )}
      </main>
    </div>
  );
}
