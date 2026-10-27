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

#[derive(Serialize)]
struct PagedResult {
    items: Vec<String>,
    total_count: usize,
}

// Mock a large dataset
fn get_mock_data() -> Vec<String> {
    (1..=100_000).map(|i| format!("Data item {}", i)).collect()
}

#[tauri::command]
fn fetch_words(query: String, offset: usize, limit: usize) -> PagedResult {
    let all_data = get_mock_data();

    // Filter based on the query
    let filtered: Vec<String> = all_data
        .into_iter()
        .filter(|w| w.to_lowercase().contains(&query.to_lowercase()))
        .collect();

    let total_count = filtered.len();

    // Paginate
    let items = filtered.into_iter().skip(offset).take(limit).collect();

    PagedResult { items, total_count }
}

#[tauri::command]
fn fetch_details(word: String) -> String {
    format!("Detailed backend information for: {}", word)
}
