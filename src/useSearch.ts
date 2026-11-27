// Live search results for the virtualized list, fetched page by page.

import { useCallback, useRef, useState } from "react";
import * as api from "./api";

const PAGE_SIZE = 100;

interface State {
  query: string;
  /** The `libraryVersion` the pages were fetched for. */
  version: number;
  /** Unknown until the first page for `query` arrives. */
  totalCount: number | null;
  /** Loaded pages by page number. Any page can load first (scrollbar drags). */
  pages: ReadonlyMap<number, string[]>;
  error: string | null;
}

export interface SearchResults {
  totalCount: number | null;
  error: string | null;
  /** The headword at `index`, or `undefined` while its page is loading. */
  item(index: number): string | undefined;
  /** Makes sure rows `first..=last` are loaded or loading. */
  loadRange(first: number, last: number): void;
}

function fresh(query: string, version: number): State {
  return { query, version, totalCount: null, pages: new Map(), error: null };
}

/**
 * Results for `query`. Bump `libraryVersion` whenever the library's contents
 * change (a dictionary was added) to fetch them again.
 */
export function useSearch(query: string, libraryVersion: number): SearchResults {
  const [state, setState] = useState(() => fresh(query, libraryVersion));
  // A new query or library discards the old results before anything renders them.
  if (state.query !== query || state.version !== libraryVersion) {
    setState(fresh(query, libraryVersion));
  }

  // Requests in flight, as "version␀page␀query" keys, so no page is fetched twice.
  const pending = useRef(new Set<string>());

  const loadRange = useCallback(
    (first: number, last: number) => {
      const { query: q, version } = state;
      const outdated = (s: State) => s.query !== q || s.version !== version;
      for (let page = Math.floor(first / PAGE_SIZE); page <= Math.floor(last / PAGE_SIZE); page++) {
        const key = `${version}\u0000${page}\u0000${q}`;
        if (state.pages.has(page) || pending.current.has(key)) continue;
        pending.current.add(key);
        api
          .search(q, page * PAGE_SIZE, PAGE_SIZE)
          .then((result) =>
            // Responses for an older query or library are dropped.
            setState((s) =>
              outdated(s)
                ? s
                : {
                    ...s,
                    totalCount: result.totalCount,
                    pages: new Map(s.pages).set(page, result.items),
                  },
            ),
          )
          .catch((e) => setState((s) => (outdated(s) ? s : { ...s, error: String(e) })))
          .finally(() => pending.current.delete(key));
      }
    },
    [state],
  );

  const item = useCallback(
    (index: number) => state.pages.get(Math.floor(index / PAGE_SIZE))?.[index % PAGE_SIZE],
    [state],
  );

  return { totalCount: state.totalCount, error: state.error, item, loadRange };
}
