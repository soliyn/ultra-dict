// Back/forward through the headwords looked up this session
// (docs/spec-v1.md, "History"). It records the word as the user looked it
// up, not where a redirect took it.

import { useCallback, useState } from "react";

// Oldest lookups beyond this are forgotten.
const MAX_ENTRIES = 500;

interface State {
  entries: string[];
  /** The current entry, or -1 before the first lookup. */
  index: number;
}

export interface History {
  /** The headword to show, or null before the first lookup. */
  current: string | null;
  canGoBack: boolean;
  canGoForward: boolean;
  /**
   * Looks `word` up: it becomes the current entry, and the entries after
   * the current one are dropped. With `replace`, it takes the current
   * entry's place instead of following it. Looking up the current word
   * again changes nothing.
   */
  open: (word: string, replace?: boolean) => void;
  /** Steps to the previous entry, if any. */
  back: () => void;
  /** Steps to the next entry, if any. */
  forward: () => void;
}

/** The session's lookup history, empty at first. */
export function useHistory(): History {
  const [{ entries, index }, setState] = useState<State>({ entries: [], index: -1 });

  const open = useCallback((word: string, replace = false) => {
    setState((state) => {
      if (state.entries[state.index] === word) return state;
      const kept = state.entries.slice(0, replace ? Math.max(state.index, 0) : state.index + 1);
      // Replacing with the word before it just steps back onto that one.
      const next = (kept[kept.length - 1] === word ? kept : [...kept, word]).slice(-MAX_ENTRIES);
      return { entries: next, index: next.length - 1 };
    });
  }, []);
  const back = useCallback(() => {
    setState((state) => (state.index > 0 ? { ...state, index: state.index - 1 } : state));
  }, []);
  const forward = useCallback(() => {
    setState((state) =>
      state.index < state.entries.length - 1 ? { ...state, index: state.index + 1 } : state,
    );
  }, []);

  return {
    current: entries[index] ?? null,
    canGoBack: index > 0,
    canGoForward: index < entries.length - 1,
    open,
    back,
    forward,
  };
}
