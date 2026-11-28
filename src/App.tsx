// The single main window: search box and results on one side, the article
// area (or the dictionary list) on the other.

import { KeyboardEvent, useCallback, useEffect, useRef, useState } from "react";
import * as api from "./api";
import { ArticleView } from "./ArticleView";
import { LibraryView } from "./LibraryView";
import { ResultsList } from "./ResultsList";
import { strings } from "./strings";
import { useHistory } from "./useHistory";
import { useSearch } from "./useSearch";
import "./App.css";

const RESULTS_ID = "results";

/** A results row to open once its headword has loaded. */
interface Opening {
  index: number;
  /** Replace the current history entry rather than add one. */
  replace: boolean;
}

// History shortcuts as each platform has them: Cmd+[ and Cmd+] on macOS
// (where Option+arrows move by word in text), Alt+arrows elsewhere. The
// keyboard's own Back and Forward keys work on all.
const IS_MAC = /Mac/i.test(navigator.platform);
const SHORTCUTS = IS_MAC ? { back: "⌘[", forward: "⌘]" } : { back: "Alt+←", forward: "Alt+→" };

/** The history step a key press asks for, if any. */
function historyKey(e: globalThis.KeyboardEvent): "back" | "forward" | null {
  if (e.key === "BrowserBack") return "back";
  if (e.key === "BrowserForward") return "forward";
  const modified = IS_MAC
    ? e.metaKey && !e.altKey && !e.ctrlKey && !e.shiftKey
    : e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey;
  if (!modified) return null;
  if (e.key === (IS_MAC ? "[" : "ArrowLeft")) return "back";
  if (e.key === (IS_MAC ? "]" : "ArrowRight")) return "forward";
  return null;
}

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

  // The headword shown is the current entry of the session history. It
  // outlives the results: typing a new query keeps it shown.
  const history = useHistory();

  // The highlighted row of the current results, while it holds the headword
  // shown, and a row waiting for its headword to load before it opens.
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [opening, setOpening] = useState<Opening | null>(null);
  const [selectionKey, setSelectionKey] = useState(resultKey);
  if (selectionKey !== resultKey) {
    setSelectionKey(resultKey);
    setSelectedIndex(null);
    setOpening(null);
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

  // Opens row `index`. Each opened row is a new history entry, except that
  // stepping through the rows with the arrow keys (`replace`) updates the
  // entry of the row stepped from rather than adding one per row.
  const select = (index: number, replace = false) => {
    setSelectedIndex(index);
    // A row still waiting to open was never added: the row replacing it
    // takes its place as a new entry.
    const stepping = selectedIndex !== null && (opening === null || opening.replace);
    setOpening({ index, replace: replace && stepping });
    setShowLibrary(false);
  };

  // The row's headword may still be loading (arrow keys across a page edge).
  const { item } = results;
  const { open } = history;
  useEffect(() => {
    if (opening === null) return;
    const headword = item(opening.index);
    if (headword === undefined) return;
    setOpening(null);
    open(headword, opening.replace);
  }, [item, opening, open]);

  // Showing a word that isn't the highlighted row's: an entry link, or a
  // step through the history.
  const leaveResults = () => {
    setSelectedIndex(null);
    setOpening(null);
    setShowLibrary(false);
  };
  const openEntry = (word: string) => {
    leaveResults();
    open(word);
  };
  const goBack = () => {
    if (!history.canGoBack) return;
    leaveResults();
    history.back();
  };
  const goForward = () => {
    if (!history.canGoForward) return;
    leaveResults();
    history.forward();
  };

  // Keyboard shortcuts and the mouse's back/forward buttons, anywhere in the
  // window. Handled here, the webview's own history must not see them.
  const latestGo = useRef({ goBack, goForward });
  useEffect(() => {
    latestGo.current = { goBack, goForward };
  });
  useEffect(() => {
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      const step = historyKey(e);
      if (step === null) return;
      e.preventDefault();
      latestGo.current[step === "back" ? "goBack" : "goForward"]();
    };
    const onMouse = (e: MouseEvent) => {
      if (e.button !== 3 && e.button !== 4) return;
      e.preventDefault();
      if (e.type === "mouseup") latestGo.current[e.button === 3 ? "goBack" : "goForward"]();
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("mousedown", onMouse);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("mousedown", onMouse);
      window.removeEventListener("mouseup", onMouse);
    };
  }, []);

  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      // Open the best match (row 0: exact case first), unless a row is
      // already open. The row may still be loading; the effect above waits.
      e.preventDefault();
      if (results.totalCount !== 0) select(selectedIndex ?? 0);
      return;
    }
    const count = results.totalCount ?? 0;
    if (count === 0 || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      select(selectedIndex === null ? 0 : Math.min(selectedIndex + 1, count - 1), true);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      select(selectedIndex === null ? 0 : Math.max(selectedIndex - 1, 0), true);
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
        ) : history.current === null ? (
          <p className="placeholder">{strings.noSelection}</p>
        ) : (
          <>
            <nav className="article-nav">
              <button
                className="button icon"
                onClick={goBack}
                disabled={!history.canGoBack}
                aria-label={strings.back}
                title={strings.backHint(SHORTCUTS.back)}
              >
                ←
              </button>
              <button
                className="button icon"
                onClick={goForward}
                disabled={!history.canGoForward}
                aria-label={strings.forward}
                title={strings.forwardHint(SHORTCUTS.forward)}
              >
                →
              </button>
            </nav>
            <ArticleView headword={history.current} libraryVersion={libraryVersion} onOpenEntry={openEntry} />
          </>
        )}
      </main>
    </div>
  );
}
