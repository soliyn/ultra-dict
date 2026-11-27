# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Package manager is **bun** (Tauri's `beforeDevCommand`/`beforeBuildCommand` call `bun run …`).

- `bun run tauri dev`: run the desktop app (starts Vite on fixed port 1420, then the Rust host).
- `bun run tauri build`: release bundle.
- `bun run build`: frontend only (`tsc` type-check + Vite build to `dist/`).
- `cargo check` / `cargo clippy` / `cargo test`: run from `src-tauri/`.
- `cargo run` / `cargo run -- --check`: run from `tools/make-fixtures/` to regenerate or verify the MDict test fixtures in `src-tauri/tests/fixtures/` (see the README there).

No test suite or linter config exists yet.

## Architecture

Tauri 2 desktop app: React 19 + TypeScript frontend (`src/`), Rust backend (`src-tauri/`). Prototype stage: the dictionary data is mocked in Rust.

**Target design:** read `docs/spec-v1.md` before any work on dictionary formats, indexing, search, article rendering or the library. It records the agreed v1 decisions and the build order. The prototype described below is what that spec replaces.

**Data flow.** The frontend never holds the full word list. `MainWindow` calls the Rust command `fetch_words(query, offset, limit)`, which filters and paginates server-side and returns `{ items, total_count }`. `total_count` sizes a `@tanstack/react-virtual` list; further pages are fetched when the last virtual row reaches the end of the loaded `items`. The `PagedResult` shape is declared on both sides (`src/App.tsx` and `src-tauri/src/lib.rs`): change them together.

**Multi-window via URL routing.** One frontend bundle serves every window. `App` reads `?word=` from the URL: present → `DetailWindow` (calls `fetch_details`), absent → `MainWindow`. Detail windows are opened from the frontend with `new WebviewWindow("detail-<timestamp>", { url: "/?word=…" })`.

**Adding a Rust command** means defining a `#[tauri::command]` fn in `src-tauri/src/lib.rs` and registering it in `generate_handler![…]`; the frontend calls it with `invoke("name", { camelCaseArgs })`.

**Capabilities.** `src-tauri/capabilities/default.json` grants permissions (including window/webview creation) only to the window labelled `main`. Detail windows (`detail-*`) are not covered, so any Tauri core/plugin API a detail window needs requires extending that capability's `windows` list.
