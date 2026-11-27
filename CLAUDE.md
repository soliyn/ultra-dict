# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Package manager is **bun** (Tauri's `beforeDevCommand`/`beforeBuildCommand` call `bun run …`).

- `bun run tauri dev`: run the desktop app (starts Vite on fixed port 1420, then the Rust host).
- `bun run tauri build`: release bundle.
- `bun run build`: frontend only (`tsc` type-check + Vite build to `dist/`).
- `cargo check` / `cargo clippy` / `cargo test`: run from `src-tauri/`.
- `ULTRA_DICT_REFERENCE_DIR=<dir> cargo test --release --test reference_set -- --ignored --nocapture`: run from `src-tauri/` to smoke-test the format layer against a local folder of real dictionaries.
- `cargo run` / `cargo run -- --check`: run from `tools/make-fixtures/` to regenerate or verify the MDict test fixtures in `src-tauri/tests/fixtures/` (see the README there).
- `ULTRA_DICT_LIBRARY=<file> bun run tauri dev`: run the app on another library database, leaving the real one (in the app data dir) untouched. Debug builds only.
- `tools/isolation-check/`: the script-isolation check against the running app (see its README). Rerun it after upgrading Tauri, wry or WebView2, or changing how article frames are built.

No test suite or linter config exists yet.

## Architecture

Tauri 2 desktop app: React 19 + TypeScript frontend (`src/`), Rust backend (`src-tauri/`). Early v1: the backend is real (Library over SQLite); the UI is one window with an add-file dialog, live search and article view, with dictionary resources served over `dict://`.

**Target design:** read `docs/spec-v1.md` before any work on dictionary formats, indexing, search, article rendering or the library. It records the agreed v1 decisions and the build order. The prototype described below is what that spec replaces.

**Formats.** `src-tauri/src/format/` defines the `Format` and `Dictionary` traits and our own types (`EntryLocator`, `Metadata`, `FormatError`). `format/mdict.rs` is the only file allowed to use `mdictlib`: keep its types and errors inside it.

**Library.** `src-tauri/src/library/` is the single seam over dictionaries, the SQLite index (`schema.sql`, versioned by `PRAGMA user_version`), search, articles and history. Callers and `tests/library.rs` use only its public types; `rusqlite` and the format layer stay behind it. The one exception is the schema-migration test, which writes an old-version database with `rusqlite` directly.

**Commands.** `src-tauri/src/commands.rs` holds the Tauri commands (`add_file`, `search`, `article`, `set_allow_scripts`). They lock the `Mutex<Library>` app state (opened in `setup` on `library.sqlite3` in the app data dir), make one Library call, and return camelCase DTOs; errors reject `invoke` with a readable string. The DTO shapes are mirrored as TypeScript interfaces, with typed wrappers, in `src/api.ts`: change both together. `tests/commands.rs` calls them through Tauri's mock IPC.

**Frontend.** One window, plain React hooks and plain CSS (`App.css`, themed with CSS variables and `prefers-color-scheme`); no router or state library. Every UI string lives in `src/strings.ts`. `App` holds the query and selection. `ResultsList` is a `@tanstack/react-virtual` list sized by `totalCount`. `ArticleView` renders each dictionary section in its own iframe sized to its content. A frame may run scripts or share the app's origin, never both. With the dictionary's "Allow scripts" off (the default), it gets `sandbox="allow-same-origin"`: nothing runs, and the parent measures it. With scripts on, it gets `sandbox="allow-scripts"` alone: the origin is opaque, a meta CSP keeps scripts and connections on the `dict` scheme, and an injected script reports the height by `postMessage`. Clicks on `sound://` links reach the app the same two ways (the parent's listener on a scripts-off frame's document, a `postMessage` from the injected script otherwise), and `src/sound.ts` plays the resource in the app window. `App` bumps a `libraryVersion` after each add, and search results and the open article refetch when it changes.

**Resources.** `src-tauri/src/protocol.rs` registers the `dict` URI scheme over `Library::resource` (`.mdd` volumes first, then loose files beside the `.mdx`; `..` and drive paths refused). URLs are `<base><dict-id>/<path>`: the base is `http://dict.localhost/` on Windows (WebView2's only form for custom schemes) and `dict://localhost/` elsewhere, so the id sits in the path, not the host. Root-relative references (`/img/a.png`) would therefore lose the id, so `src/resources.ts` rewrites each entry's resource references to absolute URLs before it goes into the frame. Every response, 404s included, carries `Access-Control-Allow-Origin: *` because fonts load with CORS from the frame's other origin. Anything unresolvable is an empty 404, never an error.

**Data flow.** The frontend never holds the full word list. `useSearch(query)` keeps a cache of fixed-size pages fetched with `search(query, offset, limit)` and loads whichever pages the visible rows need, so a scrollbar drag fetches one page, not every page before it. Responses for an outdated query are dropped.

**Adding a Rust command** means defining a `#[tauri::command]` fn in `src-tauri/src/commands.rs` and listing it in `with_commands` in `src-tauri/src/lib.rs` (shared by the app and the IPC tests); the frontend calls it with `invoke("name", { camelCaseArgs })`.

**Windows manifest.** `src-tauri/build.rs` embeds `windows-app-manifest.xml` through the linker for every target, not only the app binary, so test binaries that start Tauri's mock runtime can load. Without it they fail with `STATUS_ENTRYPOINT_NOT_FOUND`.

**WebView2 frames.** On Windows, Tauri's init scripts, `__TAURI_INTERNALS__` and its invoke key included, are injected into every iframe, scripted dictionary frames too. Isolation rests on two facts. Tauri refuses IPC whose `Origin` isn't a URL (an opaque frame sends `null`). And wry only listens to top-level `postMessage`. `tauri.conf.json` sets `additionalBrowserArgs` to disable `IsolateSandboxedIframes`, because WebView2 doesn't route an isolated frame process's first requests through custom schemes, so `dict` resources in scripted frames fail to load. The value repeats wry's default `--disable-features` list, because setting it replaces that list. Any new window (pop-out, #21) needs the same args.

**Capabilities.** `src-tauri/capabilities/default.json` grants only `core:default` and `dialog:allow-open` (the add-file picker), and only to the window labelled `main`. A feature that needs more (extra windows for pop-out, say) must add exactly those permissions and, for new windows, their labels.
