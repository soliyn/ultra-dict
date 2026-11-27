pub mod commands;
pub mod format;
pub mod library;

use std::sync::Mutex;

use tauri::{Builder, Manager, Runtime};

use crate::library::Library;

/// File name of the library database inside the app data directory.
const LIBRARY_FILE: &str = "library.sqlite3";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    with_commands(Builder::default())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = app.path().app_data_dir()?.join(LIBRARY_FILE);
            let library = Library::open(&path)
                .map_err(|e| format!("cannot open the library at {}: {e}", path.display()))?;
            app.manage(Mutex::new(library));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Registers every frontend-callable command. The app's state must include
/// a [`commands::SharedLibrary`].
///
/// Adding a command: write it in [`commands`] and list it here.
pub fn with_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::add_file,
        commands::search,
        commands::article,
    ])
}
