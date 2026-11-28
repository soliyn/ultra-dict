//! The Tauri commands, called through the IPC layer of Tauri's mock runtime
//! exactly as the frontend's `invoke(name, args)` would: argument names,
//! result JSON and error strings included.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{App, Listener, WebviewWindow, WebviewWindowBuilder};
use tempfile::TempDir;
use ultra_dict_lib::commands::INDEXING_EVENT;
use ultra_dict_lib::library::Library;

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

struct TestApp {
    _dir: TempDir,
    _app: App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
    /// Payloads of the indexer's events, as the frontend receives them.
    indexing: Receiver<Value>,
}

impl TestApp {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(&dir.path().join("library.sqlite3")).unwrap();
        let app = ultra_dict_lib::with_commands(mock_builder())
            .manage(Mutex::new(library))
            .build(mock_context(noop_assets()))
            .expect("build app");
        let (send, indexing) = mpsc::channel();
        app.listen_any(INDEXING_EVENT, move |event| {
            let _ = send.send(serde_json::from_str(event.payload()).unwrap());
        });
        ultra_dict_lib::start_indexer(app.handle()).expect("start indexer");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        Self { _dir: dir, _app: app, webview, indexing }
    }

    /// Waits for the indexer to finish with dictionary `id` and returns the
    /// events it sent until then.
    fn wait_for_indexing(&self, id: &Value) -> Vec<Value> {
        let mut seen = Vec::new();
        loop {
            let event = self.indexing.recv_timeout(Duration::from_secs(30)).expect("indexing event");
            let done = event["dictionaryId"] == *id && event["finished"] == true;
            seen.push(event);
            if done {
                return seen;
            }
        }
    }

    /// Dictionary `id` as `dictionaries` lists it.
    fn dictionary(&self, id: &Value) -> Value {
        let list = self.invoke("dictionaries", json!({})).unwrap();
        list.as_array().unwrap().iter().find(|d| d["id"] == *id).expect("listed").clone()
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

    /// Adds fixture `rel`, waits until it is indexed, and returns it.
    fn add(&self, rel: &str) -> Value {
        let added = self
            .invoke("add_file", json!({ "path": fixture(rel) }))
            .unwrap_or_else(|e| panic!("add_file {rel}: {e}"));
        self.wait_for_indexing(&added["id"]);
        self.dictionary(&added["id"])
    }
}

#[test]
fn add_file_returns_the_dictionary_and_indexes_it_in_the_background() {
    let app = TestApp::new();
    let added = app
        .invoke("add_file", json!({ "path": fixture("basic/basic.mdx") }))
        .unwrap();
    assert!(added["id"].is_i64(), "{added}");
    assert_eq!(added["status"], "indexing");
    assert_eq!(added["title"], "basic");
    let id = added["id"].clone();
    let events = app.wait_for_indexing(&id);
    assert_eq!(
        events,
        [
            json!({ "dictionaryId": id, "finished": false, "done": 0, "total": 19 }),
            json!({ "dictionaryId": id, "finished": true, "done": 0, "total": 0 }),
        ]
    );

    let info = app.dictionary(&id);
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
    assert_eq!(added[0]["status"], "indexing");
    app.wait_for_indexing(&added[0]["id"]);
    let info = app.dictionary(&added[0]["id"]);
    assert_eq!(info["title"], "Multi-volume Test Dictionary");
    assert_eq!(info["status"], "ready");
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

#[test]
fn reindex_and_cancel_commands() {
    let app = TestApp::new();
    let basic = app.add("basic/basic.mdx");
    let id = basic["id"].clone();

    let reindexing = app.invoke("reindex_dictionary", json!({ "dictionaryId": id })).unwrap();
    assert_eq!(reindexing["status"], "indexing");
    assert_eq!(reindexing["headwordCount"], 0);
    app.wait_for_indexing(&id);
    assert_eq!(app.dictionary(&id), basic);

    // Nothing under way: cancelling changes nothing.
    let info = app.invoke("cancel_indexing", json!({ "dictionaryId": id })).unwrap();
    assert_eq!(info, basic);

    for cmd in ["reindex_dictionary", "cancel_indexing"] {
        let error = app.invoke(cmd, json!({ "dictionaryId": 999 })).unwrap_err();
        assert_eq!(error, "no dictionary with id 999 in the library", "{cmd}");
    }
}

#[test]
fn relocate_dictionary_points_it_at_another_file() {
    let app = TestApp::new();
    let id = app.add("basic/basic.mdx")["id"].clone();
    app.invoke("rename_dictionary", json!({ "dictionaryId": id, "name": "Mine" })).unwrap();

    let path = fixture("multivolume/multi.mdx");
    let relocating = app
        .invoke("relocate_dictionary", json!({ "dictionaryId": id, "path": path }))
        .unwrap();
    assert_eq!(relocating["status"], "indexing");
    assert_eq!(Path::new(relocating["path"].as_str().unwrap()), path);
    app.wait_for_indexing(&id);
    let info = app.dictionary(&id);
    assert_eq!((&info["status"], &info["title"]), (&json!("ready"), &json!("Mine")));

    let untitled = app.add("untitled/untitled.mdx");
    let error = app
        .invoke("relocate_dictionary", json!({ "dictionaryId": id, "path": untitled["path"] }))
        .unwrap_err();
    let taken = untitled["path"].as_str().unwrap();
    assert_eq!(error, format!("{taken} is already in the library as \"untitled\""));
}
