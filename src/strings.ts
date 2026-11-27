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
  addFile: "Add file…",
  adding: "Adding…",
  dictionaryFilter: "MDict dictionaries",
  addFailed: (message: string) => `Couldn't add the dictionary: ${message}`,
  dictionaryUnusable: (title: string, message: string) => `${title} can't be used: ${message}`,
  dismiss: "Dismiss",
  noArticle: "No dictionary has this word any more.",
  articleFailed: (message: string) => `Couldn't open the article: ${message}`,
  sectionFailed: (message: string) => `Couldn't read this dictionary: ${message}`,
  allowScripts: "Allow scripts",
  allowScriptsHint: "Run this dictionary's own JavaScript in its articles, isolated from the app.",
  settingFailed: (message: string) => `Couldn't change the setting: ${message}`,
} as const;
