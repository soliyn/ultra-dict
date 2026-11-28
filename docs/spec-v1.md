# ultra-dict v1 spec

A desktop dictionary app: the user adds dictionary files of various formats and looks words up across all of them at once. v1 supports MDict (`.mdx` + `.mdd`); the architecture must let further formats (StarDict, DSL, Lingvo `.lsa` sound archives, …) be added without touching search or UI.

## Scope

**In v1:** desktop (Windows first; macOS and Linux kept buildable), English UI, MDict format, everything below.

**Out of v1** (design must not preclude them): mobile; system-wide hotkey / clipboard / tray lookup; dictionary groups (e.g. per language pair); favourites; registration-key encrypted dictionaries; Speex (`.spx`) audio; dark mode and stylesheet picker for articles; contains/fuzzy search beyond "did you mean"; localisation (all UI strings live in one module so it can be added).

License: MIT.

## Formats

- Formats are a Rust trait compiled into the app, one module per format. The trait covers: open + metadata (title, description), headword enumeration (for indexing), entry lookup by locator, and resource lookup by path.
- No runtime plugin system.

### MDict

- Parsed with the `mdictlib` crate, 0.2.8 pinned exactly, `lzo` feature enabled. It passed on all 25 local dictionaries (issue #2).
- **Replaceable layer:** `mdictlib` is used only inside the MDict adapter. No `mdictlib` type, error or concept leaks out of it; the entry locator stored in SQLite is our own opaque value (physical position in the file). The Library tests drive only our interfaces, so they double as the acceptance suite for a replacement reader.
- **Release blocker:** `mdictlib`'s MIT license is in doubt. About half of `mdict-rs`'s (AGPL-3.0) distinctive source lines appear verbatim in it, with no attribution (#2). Before any public release or distribution, replace it with our own MIT reader (written from format documentation, not from GPL/AGPL code such as GoldenDict-ng or `readmdict`), or get its licensing confirmed by the authors.
- Supported: MDX v1.2 and v2.0; zlib and LZO; key-index encryption (`Encrypted="2"`). MDX 3.0 is not supported.
- Dictionaries requiring a registration key (`Encrypted="1"`) are rejected with a clear "unsupported: needs registration key" error.
- Companion resources: `name.mdd`, `name.1.mdd`, `name.2.mdd`, … next to the `.mdx` are discovered automatically, searched in volume order.

## Library

- **Adding:** "Add file" (one `.mdx`) and "Add folder" (scanned on add; rescan on demand). Files are referenced in place, never copied. Adding an already-present path is a no-op.
- **Display name:** MDX `Title` with HTML stripped, falling back to the filename. The user can rename; the rename survives reindexing.
- **Organisation:** a flat list; each dictionary can be enabled/disabled and reordered. The order drives article order in lookups.
- **Status per dictionary:** `indexing (progress)`, `ready`, `needs reindex`, `missing`, `error (message)`.
- **Startup check:** compare each file's size and mtime with the indexed values. Changed → reindex in the background. Gone → `missing`, with a "re-locate" action. No filesystem watching.

## Index and storage

One SQLite database (`rusqlite`, bundled) in the Tauri app data directory holds:

- **Headwords:** original headword, folded key (lowercase + diacritics stripped), dictionary id, format-specific locator for the entry.
- **Library:** path, display name, user rename, order, enabled flag, size/mtime at index time, status.
- **History:** recent lookups.

Indexing:

- Each dictionary is indexed once, when added (or when it changes). Startup only reads SQLite; the format reader is opened lazily to fetch articles and resources.
- Runs on a background thread, emitting progress events to the UI, and is cancellable. Headwords are written in short batches; the dictionary turns `ready` in the last one.
- A dictionary is searchable only once indexing completes. Interrupted or cancelled indexing leaves it `needs reindex`, never half-indexed.
- The largest reference dictionary (500 MB `.mdx`) is the performance benchmark for indexing time: about 0.7 s for its ~250k headwords in a release build (#16).

## Search

- Prefix match on the folded key across all enabled, ready dictionaries.
- The result list shows only matches (not the whole alphabet scrolled to a position), sorted case-insensitively, paged into the virtualized list.
- One row per exact headword spelling. "Polish" and "polish" are separate rows; for a query, the exact-case match sorts first.
- No matches → "did you mean" suggestions.

## Articles

### Layout and navigation

- A single main window: search box and headword list on one side, article view on the other.
- Back/forward history within the session.
- Optional "pop out" opens the current article in its own window. This replaces the prototype's window-per-entry flow.

### Composition

- Opening a headword shows one section per enabled dictionary that has it, in library order.
- Multiple entries for the same headword in one dictionary all appear in that section, in file order.
- When other case/diacritic variants of the headword exist, the view adds a "see also" link to them.
- `@@@LINK=target` redirects are followed automatically, up to 5 hops (loop guard). The article header shows the redirect ("went → go"); history records the word the user looked up.

### Rendering

- Each dictionary section renders in its own sandboxed iframe on a white background (dictionary CSS assumes white; the app chrome follows the system light/dark theme).
- **Resources:** a custom URI scheme `dict` serves CSS, JS, images, fonts and audio at `<base><dict-id>/<path>`.
  - The base is `dict://localhost/`, or `http://dict.localhost/` on Windows, where WebView2 fixes the host. So the id is a path segment, and article HTML is rewritten to absolute resource URLs (root-relative `/img/a.png` included).
  - Known limitation: inside a served stylesheet, a root-relative `url(/…)` or a `../` climbing above the dictionary root loses the id and is not found. The reference set has this only in leftover web paths.
  - Resolution order: the dictionary's `.mdd` volumes, then loose files in the dictionary's folder. Many dictionaries ship CSS/JS beside the `.mdx`, and some have no `.mdd` at all.
  - Paths escaping the folder (`..`) are refused.
- **Stylesheets:** articles get whatever stylesheet they link to (no picker in v1, even where a dictionary ships alternative stylesheets).
- **Links:** `entry://word` (and equivalent in-dictionary links) become in-app lookups and push history.
- **Sound:** `sound://…` links play audio the webview supports (mp3, wav, ogg). Other formats show "can't play this format".
- **Scripts:** dictionary JS is controlled by a per-dictionary "Allow scripts" toggle, default off. Enabled scripts run in a sandboxed, opaque-origin iframe. **Requirement:** a dictionary script must not be able to reach the Tauri IPC / backend commands. This is verified in the first slice.
  - Verified on Windows (issue #12) with `tools/isolation-check`, which reruns the check against the live app. Tauri injects its internals into every frame, but it refuses IPC from the opaque origin, and a frame's `postMessage` never reaches it.
  - The app disables Chromium's `IsolateSandboxedIframes`. WebView2 doesn't route an isolated frame process's first requests through custom schemes, so scripted frames would lose their `dict` resources. The opaque origin still provides the isolation.

## History

- Session back/forward (above).
- Persistent recent-lookups list in SQLite, capped at ~1,000 entries, user-clearable.

## Frontend

Plain React with hooks and plain CSS; no router or state library until a concrete need appears. The prototype's inline styles move to CSS and its mock data is removed.

## Testing

- **Automated (Rust, `cargo test`):** format adapter, index (folding, prefix search, exact-case ordering, redirects and loop guard), resource resolution (mdd → folder, `..` refusal). They run against one or two tiny, freely licensed sample dictionaries committed to the repo.
- **Manual reference set** (the developer's own dictionaries; local only, never committed). It covers these roles:
  - a small dictionary that ships JS
  - one with multi-volume `.mdd`
  - one with no `.mdd` and loose CSS
  - a large community dictionary
  - a ~500 MB `.mdx` with a multi-GB `.mdd` and alternative stylesheets (the indexing benchmark)
- No frontend tests in v1; add Vitest once TypeScript-side logic warrants it.

## Build order

1. **Thin end-to-end slice:**
   1. Add one `.mdx`.
   2. Index it into SQLite.
   3. Prefix search.
   4. Render an article with `.mdd` resources via `dict://`.

   This slice also runs the script-isolation check, and it replaces the prototype's mock data and window-per-entry flow.
2. **Widen:** add folder, enable/disable and ordering, redirects, multiple dictionaries per article, "see also", "did you mean", sound, scripts toggle, history (session + persistent), pop-out, startup change detection and reindex, missing/re-locate, renaming.
