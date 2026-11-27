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

No test suite or linter config exists yet.

## Architecture

Tauri 2 desktop app: React 19 + TypeScript frontend (`src/`), Rust backend (`src-tauri/`). Prototype stage: the backend is real (Library over SQLite), the UI is still the prototype's.

**Target design:** read `docs/spec-v1.md` before any work on dictionary formats, indexing, search, article rendering or the library. It records the agreed v1 decisions and the build order. The prototype described below is what that spec replaces.

**Formats.** `src-tauri/src/format/` defines the `Format` and `Dictionary` traits and our own types (`EntryLocator`, `Metadata`, `FormatError`). `format/mdict.rs` is the only file allowed to use `mdictlib`: keep its types and errors inside it.

**Library.** `src-tauri/src/library/` is the single seam over dictionaries, the SQLite index (`schema.sql`, versioned by `PRAGMA user_version`), search, articles and history. Callers and `tests/library.rs` use only its public types; `rusqlite` and the format layer stay behind it.

**Commands.** `src-tauri/src/commands.rs` holds the Tauri commands (`add_file`, `search`, `article`). They lock the `Mutex<Library>` app state (opened in `setup` on `library.sqlite3` in the app data dir), make one Library call, and return camelCase DTOs; errors reject `invoke` with a readable string. The DTO shapes are mirrored as TypeScript interfaces in `src/App.tsx`: change both together. `tests/commands.rs` calls them through Tauri's mock IPC.

**Data flow.** The frontend never holds the full word list. `MainWindow` calls `search(query, offset, limit)`, which pages server-side and returns `{ items, totalCount }`. `totalCount` sizes a `@tanstack/react-virtual` list; further pages are fetched when the last virtual row reaches the end of the loaded `items`.

**Multi-window via URL routing.** One frontend bundle serves every window. `App` reads `?word=` from the URL: present → `DetailWindow` (calls `article`, shows entry HTML as plain text), absent → `MainWindow`. Detail windows are opened from the frontend with `new WebviewWindow("detail-<timestamp>", { url: "/?word=…" })`.

**Adding a Rust command** means defining a `#[tauri::command]` fn in `src-tauri/src/commands.rs` and listing it in `with_commands` in `src-tauri/src/lib.rs` (shared by the app and the IPC tests); the frontend calls it with `invoke("name", { camelCaseArgs })`.

**Windows manifest.** `src-tauri/build.rs` embeds `windows-app-manifest.xml` through the linker for every target, not only the app binary, so test binaries that start Tauri's mock runtime can load. Without it they fail with `STATUS_ENTRYPOINT_NOT_FOUND`.

**Capabilities.** `src-tauri/capabilities/default.json` grants permissions (including window/webview creation) only to the window labelled `main`. Detail windows (`detail-*`) are not covered, so any Tauri core/plugin API a detail window needs requires extending that capability's `windows` list.
