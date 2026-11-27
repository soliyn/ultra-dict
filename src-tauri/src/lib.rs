pub mod format;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![fetch_words, fetch_details])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

use serde::Serialize;
use std::sync::LazyLock;

#[derive(Serialize)]
struct PagedResult {
    items: Vec<String>,
    total_count: usize,
}

// Mock a large dataset
fn get_mock_data() -> Vec<String> {
    (1..=100_000).map(|i| format!("Data item {}", i)).collect()
}

static WORDS: LazyLock<Vec<String>> = LazyLock::new(get_mock_data);

#[tauri::command]
fn fetch_words(query: String, offset: usize, limit: usize) -> PagedResult {
    // Filter based on the query
    let query = query.to_lowercase();
    let filtered: Vec<&String> = WORDS
        .iter()
        .filter(|w| w.to_lowercase().contains(&query))
        .collect();

    let total_count = filtered.len();

    // Paginate
    let items = filtered.into_iter().skip(offset).take(limit).cloned().collect();

    PagedResult { items, total_count }
}

#[tauri::command]
fn fetch_details(word: String) -> String {
    format!("Detailed backend information for: {}", word)
}
