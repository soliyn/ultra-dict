// The virtualized list of matching headwords.

import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { strings } from "./strings";
import type { SearchResults } from "./useSearch";

/** Must match `.result-row` height in App.css. */
const ROW_HEIGHT = 30;

interface Props {
  id: string;
  results: SearchResults;
  /** Changes whenever the result set is replaced; scrolls back to the top. */
  resultKey: string;
  selectedIndex: number | null;
  onSelect(index: number): void;
}

export function ResultsList({ id, results, resultKey, selectedIndex, onSelect }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: results.totalCount ?? 0,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  });
  const rows = virtualizer.getVirtualItems();
  const first = rows[0]?.index ?? 0;
  const last = rows[rows.length - 1]?.index ?? 0;

  // Fetch whatever the visible rows need (page 0 before the count is known).
  const { loadRange } = results;
  useEffect(() => {
    loadRange(first, last);
  }, [loadRange, first, last]);

  useEffect(() => {
    virtualizer.scrollToOffset(0);
  }, [virtualizer, resultKey]);

  useEffect(() => {
    if (selectedIndex !== null) virtualizer.scrollToIndex(selectedIndex);
  }, [virtualizer, selectedIndex]);

  let message: string | null = null;
  if (results.error !== null) message = strings.searchFailed(results.error);
  else if (results.totalCount === 0) message = resultKey === "" ? strings.emptyLibrary : strings.noMatches;

  return (
    <div ref={scrollRef} className="results" id={id} role="listbox" aria-label={strings.resultsLabel}>
      {message !== null ? (
        <p className="results-message">{message}</p>
      ) : (
        // Positioning comes from the virtualizer, so it stays inline.
        <div className="results-canvas" style={{ height: virtualizer.getTotalSize() }}>
          {rows.map((row) => {
            const headword = results.item(row.index);
            const selected = row.index === selectedIndex;
            return (
              <div
                key={row.index}
                role="option"
                aria-selected={selected}
                className={"result-row" + (selected ? " selected" : "")}
                style={{ transform: `translateY(${row.start}px)` }}
                onMouseDown={(e) => e.preventDefault() /* keep focus in the search box */}
                onClick={() => headword !== undefined && onSelect(row.index)}
              >
                {headword ?? strings.loadingRow}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
