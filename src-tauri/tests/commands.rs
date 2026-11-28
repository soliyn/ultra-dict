//! The Tauri commands, called through the IPC layer of Tauri's mock runtime
//! exactly as the frontend's `invoke(name, args)` would: argument names,
//! result JSON and error strings included.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use tempfile::TempDir;
use ultra_dict_lib::library::Library;

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

struct TestApp {
    _dir: TempDir,
    _app: App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
}

impl TestApp {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(&dir.path().join("library.sqlite3")).unwrap();
        let app = ultra_dict_lib::with_commands(mock_builder())
            .manage(Mutex::new(library))
            .build(mock_context(noop_assets()))
            .expect("build app");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        Self { _dir: dir, _app: app, webview }
    }

    /// `invoke(cmd, args)`: the resolved value, or the rejected one.
    fn invoke(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let url = if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" };
        get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: url.parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_owned(),
            },
        )
        .map(|body| body.deserialize::<Value>().unwrap())
    }

    fn add(&self, rel: &str) -> Value {
        self.invoke("add_file", json!({ "path": fixture(rel) }))
            .unwrap_or_else(|e| panic!("add_file {rel}: {e}"))
    }
}

#[test]
fn add_file_returns_the_dictionary() {
    let app = TestApp::new();
    let info = app.add("basic/basic.mdx");
    assert!(info["id"].is_i64(), "{info}");
    assert_eq!(Path::new(info["path"].as_str().unwrap()), fixture("basic/basic.mdx"));
    assert_eq!(info["title"], "Basic Test Dictionary");
    assert_eq!(info["position"], 0);
    assert_eq!(info["enabled"], true);
    assert_eq!(info["status"], "ready");
    assert_eq!(info["error"], Value::Null);
    assert_eq!(info["allowScripts"], false);
    assert_eq!(info["headwordCount"], 19);
}

#[test]
fn unusable_dictionary_is_added_with_a_readable_error() {
    let app = TestApp::new();
    let info = app.add("encrypted/encrypted.mdx");
    assert_eq!(info["status"], "error");
    assert_eq!(info["error"], "unsupported: needs registration key");
}

#[test]
fn search_returns_a_page_and_the_total() {
    let app = TestApp::new();
    app.add("basic/basic.mdx");
    let page = app
        .invoke("search", json!({ "query": "pol", "offset": 0, "limit": 1 }))
        .unwrap();
    assert_eq!(page, json!({ "items": ["Polish"], "totalCount": 2 }));
}

#[test]
fn article_returns_sections_with_entries() {
    let app = TestApp::new();
    let id = app.add("basic/basic.mdx")["id"].clone();
    let article = app.invoke("article", json!({ "headword": "bank" })).unwrap();
    let section = &article["sections"][0];
    assert_eq!(section["dictionaryId"], id);
    assert_eq!(section["dictionaryTitle"], "Basic Test Dictionary");
    assert_eq!(section["error"], Value::Null);
    assert_eq!(section["allowScripts"], false);
    let entries = section["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].as_str().unwrap().contains("<sup>1</sup>"));

    let unknown = app.invoke("article", json!({ "headword": "nope" })).unwrap();
    assert_eq!(unknown, json!({ "sections": [] }));
}

#[test]
fn bad_arguments_reject_with_a_message() {
    let app = TestApp::new();
    let error = app.invoke("search", json!({ "query": "a" })).unwrap_err();
    let message = error.as_str().unwrap_or_default();
    assert!(message.contains("offset"), "{error}");
}

#[test]
fn set_allow_scripts_returns_the_updated_dictionary() {
    let app = TestApp::new();
    let id = app.add("basic/basic.mdx")["id"].clone();
    let info = app
        .invoke("set_allow_scripts", json!({ "dictionaryId": id, "allow": true }))
        .unwrap();
    assert_eq!(info["allowScripts"], true);
    let article = app.invoke("article", json!({ "headword": "go" })).unwrap();
    assert_eq!(article["sections"][0]["allowScripts"], true);

    let error = app
        .invoke("set_allow_scripts", json!({ "dictionaryId": 999, "allow": true }))
        .unwrap_err();
    assert_eq!(error, "no dictionary with id 999 in the library");
}

#[test]
fn dictionaries_lists_the_library_in_order() {
    let app = TestApp::new();
    assert_eq!(app.invoke("dictionaries", json!({})).unwrap(), json!([]));
    let basic = app.add("basic/basic.mdx");
    let encrypted = app.add("encrypted/encrypted.mdx");
    assert_eq!(app.invoke("dictionaries", json!({})).unwrap(), json!([basic, encrypted]));
    assert_eq!(basic["fileTitle"], "Basic Test Dictionary");
}

#[test]
fn folder_commands() {
    let app = TestApp::new();
    assert_eq!(app.invoke("folders", json!({})).unwrap(), json!([]));

    let scan = app.invoke("add_folder", json!({ "path": fixture("multivolume") })).unwrap();
    let folder = scan["folder"].clone();
    assert!(folder["id"].is_i64(), "{scan}");
    assert_eq!(
        std::fs::canonicalize(folder["path"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(fixture("multivolume")).unwrap()
    );
    let added = scan["added"].as_array().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["title"], "Multi-volume Test Dictionary");
    assert_eq!(added[0]["status"], "ready");
    assert_eq!(app.invoke("folders", json!({})).unwrap(), json!([folder]));

    let rescan = app.invoke("rescan_folder", json!({ "folderId": folder["id"] })).unwrap();
    assert_eq!(rescan, json!({ "folder": folder, "added": [] }));

    let error = app.invoke("rescan_folder", json!({ "folderId": 999 })).unwrap_err();
    assert_eq!(error, "no folder with id 999 in the library");
    let error = app
        .invoke("add_folder", json!({ "path": fixture("no-such-folder") }))
        .unwrap_err();
    assert!(error.as_str().unwrap().starts_with("cannot read folder"), "{error}");

    let remaining = app.invoke("remove_folder", json!({ "folderId": folder["id"] })).unwrap();
    assert_eq!(remaining, json!([]));
    assert_eq!(app.invoke("dictionaries", json!({})).unwrap().as_array().unwrap().len(), 1);
}

#[test]
fn library_management_commands() {
    let app = TestApp::new();
    let basic = app.add("basic/basic.mdx");
    let id = basic["id"].clone();
    let untitled_id = app.add("untitled/untitled.mdx")["id"].clone();

    let info = app.invoke("set_enabled", json!({ "dictionaryId": id, "enabled": false })).unwrap();
    assert_eq!(info["enabled"], false);
    let page = app.invoke("search", json!({ "query": "go", "offset": 0, "limit": 10 })).unwrap();
    assert_eq!(page["totalCount"], 0);

    let info = app.invoke("rename_dictionary", json!({ "dictionaryId": id, "name": "Mine" })).unwrap();
    assert_eq!((&info["title"], &info["fileTitle"]), (&json!("Mine"), &json!("Basic Test Dictionary")));

    let listed = app
        .invoke("move_dictionary", json!({ "dictionaryId": untitled_id, "position": 0 }))
        .unwrap();
    let order: Vec<_> = listed.as_array().unwrap().iter().map(|d| d["id"].clone()).collect();
    assert_eq!(order, [untitled_id.clone(), id.clone()]);

    let listed = app.invoke("remove_dictionary", json!({ "dictionaryId": untitled_id })).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], id);
    assert_eq!(listed[0]["position"], 0);

    let error = app.invoke("remove_dictionary", json!({ "dictionaryId": 999 })).unwrap_err();
    assert_eq!(error, "no dictionary with id 999 in the library");
}
