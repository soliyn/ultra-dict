pub mod commands;
pub mod format;
pub mod library;
pub mod protocol;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tauri::{Builder, Manager, Runtime};

use crate::library::Library;

/// The library as Tauri state, shared by the commands and the `dict` scheme.
pub type SharedLibrary = Mutex<Library>;

/// Locks the shared library. A panic mid-call can't leave the database
/// inconsistent (writes are transactional), so keep serving after one.
pub(crate) fn lock_library(library: &SharedLibrary) -> MutexGuard<'_, Library> {
    library.lock().unwrap_or_else(PoisonError::into_inner)
}

/// File name of the library database inside the app data directory.
const LIBRARY_FILE: &str = "library.sqlite3";

/// Environment variable naming another library database to open instead,
/// so manual checks can run without touching the user's own library.
const LIBRARY_ENV: &str = "ULTRA_DICT_LIBRARY";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    protocol::register(with_commands(Builder::default()))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = match std::env::var_os(LIBRARY_ENV) {
                Some(path) => PathBuf::from(path),
                None => app.path().app_data_dir()?.join(LIBRARY_FILE),
            };
            let library = Library::open(&path)
                .map_err(|e| format!("cannot open the library at {}: {e}", path.display()))?;
            app.manage(Mutex::new(library));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Registers every frontend-callable command. The app's state must include
/// a [`SharedLibrary`].
///
/// Adding a command: write it in [`commands`] and list it here.
pub fn with_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::add_file,
        commands::search,
        commands::article,
        commands::set_allow_scripts,
    ])
}
