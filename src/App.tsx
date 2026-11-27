import { useEffect, useState, useRef, FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useVirtualizer } from "@tanstack/react-virtual";
import "./App.css";

// Mirrors `SearchPage` in src-tauri/src/commands.rs.
interface SearchPage {
  items: string[];
  totalCount: number;
}

// Mirrors `Article` / `ArticleSection` in src-tauri/src/commands.rs.
interface ArticleSection {
  dictionaryId: number;
  dictionaryTitle: string;
  entries: string[];
  error: string | null;
}

interface Article {
  sections: ArticleSection[];
}

function MainWindow() {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<string[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const parentRef = useRef<HTMLDivElement>(null);

  const fetchPage = async (searchQuery: string, offset: number) => {
    const res = await invoke<SearchPage>("search", {
      query: searchQuery,
      offset,
      limit: 50, // Load 50 items per page
    });
    setTotalCount(res.totalCount);
    return res.items;
  };

  // Initial load and filter change
  useEffect(() => {
    fetchPage(query, 0).then(setItems);
  }, [query]);

  const openDetailWindow = (word: string) => {
    if (!word) return;
    const windowLabel = `detail-${Date.now()}`;
    new WebviewWindow(windowLabel, {
      url: `/?word=${encodeURIComponent(word)}`,
      title: `Details: ${word}`,
      width: 600,
      height: 400,
    });
  };

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    openDetailWindow(query || items[0]); // Open typed word or first result
  };

  const rowVirtualizer = useVirtualizer({
    count: totalCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 35, // Height of each row in px
  });

  // Load more data as the user scrolls down
  useEffect(() => {
    const virtualItems = rowVirtualizer.getVirtualItems();
    const lastVisibleItem = virtualItems[virtualItems.length - 1];
    if (!lastVisibleItem) return;

    if (lastVisibleItem.index >= items.length - 1 && items.length < totalCount) {
      fetchPage(query, items.length).then((newItems) => {
        setItems((prev) => [...prev, ...newItems]);
      });
    }
  }, [rowVirtualizer.getVirtualItems(), items.length, totalCount, query]);

  return (
    <div style={{ padding: 20, display: "flex", flexDirection: "column", height: "100vh" }}>
      <form onSubmit={handleSubmit} style={{ display: "flex", gap: 10, marginBottom: 20 }}>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter words..."
          style={{ flex: 1, padding: "8px" }}
        />
        <button type="submit">Open Details</button>
      </form>

      {/* Virtualized List Container */}
      <div
        ref={parentRef}
        style={{ flex: 1, overflow: "auto", border: "1px solid #ccc" }}
      >
        <div
          style={{
            height: `${rowVirtualizer.getTotalSize()}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {rowVirtualizer.getVirtualItems().map((virtualRow) => {
            const item = items[virtualRow.index];
            return (
              <div
                key={virtualRow.index}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: `${virtualRow.size}px`,
                  transform: `translateY(${virtualRow.start}px)`,
                  padding: "8px",
                  cursor: "pointer",
                  borderBottom: "1px solid #eee"
                }}
                onClick={() => item && openDetailWindow(item)}
              >
                {item || "Loading..."}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// Interim view until the article ticket: entry HTML is shown as plain text,
// never rendered (unsandboxed dictionary HTML could reach the IPC).
function DetailWindow({ word }: { word: string }) {
  const [article, setArticle] = useState<Article | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<Article>("article", { headword: word })
      .then(setArticle)
      .catch((e) => setError(String(e)));
  }, [word]);

  return (
    <div style={{ padding: 20 }}>
      <h2>{word}</h2>
      {error && <p>{error}</p>}
      {!article && !error && <p>Loading...</p>}
      {article?.sections.length === 0 && <p>Not found.</p>}
      {article?.sections.map((section) => (
        <section key={section.dictionaryId}>
          <h3>{section.dictionaryTitle}</h3>
          {section.error && <p>{section.error}</p>}
          {section.entries.map((entry, i) => (
            <pre key={i} style={{ whiteSpace: "pre-wrap" }}>{entry}</pre>
          ))}
        </section>
      ))}
    </div>
  );
}

// Root component routes based on URL params
export default function App() {
  const params = new URLSearchParams(window.location.search);
  const word = params.get("word");

  return word ? <DetailWindow word={word} /> : <MainWindow />;
}