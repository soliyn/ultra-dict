// Every user-visible UI string lives here, so the app can be localised later
// (docs/spec-v1.md, "Scope"). Components never hard-code text.

export const strings = {
  searchLabel: "Search headwords",
  searchPlaceholder: "Search",
  resultsLabel: "Matching headwords",
  loadingRow: "…",
  noMatches: "No matches",
  emptyLibrary: "No dictionaries yet",
  searchFailed: (message: string) => `Search failed: ${message}`,
  matchCount: (count: number) =>
    count === 1 ? "1 match" : `${count.toLocaleString("en")} matches`,
  noSelection: "Select a word to see its article.",
} as const;
