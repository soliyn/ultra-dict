import { useEffect, useState, useRef, FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useVirtualizer } from "@tanstack/react-virtual";
import "./App.css";

interface PagedResult {
  items: string[];
  total_count: number;
}

function MainWindow() {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<string[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const parentRef = useRef<HTMLDivElement>(null);

  const fetchPage = async (searchQuery: string, offset: number) => {
    const res = await invoke<PagedResult>("fetch_words", {
      query: searchQuery,
      offset,
      limit: 50, // Load 50 items per page
    });
    setTotalCount(res.total_count);
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
    const lastVisibleItem = rowVirtualizer.getVirtualItems().at(-1);
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

function DetailWindow({ word }: { word: string }) {
  const [details, setDetails] = useState("Loading...");

  useEffect(() => {
    invoke<string>("fetch_details", { word }).then(setDetails);
  }, [word]);

  return (
    <div style={{ padding: 20 }}>
      <h2>{word}</h2>
      <p>{details}</p>
    </div>
  );
}

// Root component routes based on URL params
export default function App() {
  const params = new URLSearchParams(window.location.search);
  const word = params.get("word");

  return word ? <DetailWindow word={word} /> : <MainWindow />;
}