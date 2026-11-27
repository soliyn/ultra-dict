//! The `dict` URI scheme's responses, for URL paths as the webview sends them.

use std::path::{Path, PathBuf};

use tauri::http::{Response, StatusCode, header};
use tempfile::TempDir;
use ultra_dict_lib::library::Library;
use ultra_dict_lib::protocol::respond;

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

/// A library holding `basic` and `loose-css`, and their ids.
fn library() -> (TempDir, Library, i64, i64) {
    let dir = tempfile::tempdir().unwrap();
    let mut library = Library::open(&dir.path().join("library.sqlite3")).unwrap();
    let basic = library.add_file(&fixture("basic/basic.mdx")).unwrap().id.to_raw();
    let loose = library.add_file(&fixture("loose-css/loose.mdx")).unwrap().id.to_raw();
    (dir, library, basic, loose)
}

fn header_of(response: &Response<Vec<u8>>, name: header::HeaderName) -> Option<&str> {
    response.headers().get(name).map(|v| v.to_str().unwrap())
}

#[test]
fn found_resources_are_served_with_their_type() {
    let (_dir, library, basic, loose) = library();

    let png = respond(&library, &format!("/{basic}/img/dot.png"));
    assert_eq!(png.status(), StatusCode::OK);
    assert_eq!(header_of(&png, header::CONTENT_TYPE), Some("image/png"));
    assert!(png.body().starts_with(b"\x89PNG"));

    let css = respond(&library, &format!("/{loose}/loose.css"));
    assert_eq!(css.status(), StatusCode::OK);
    assert_eq!(header_of(&css, header::CONTENT_TYPE), Some("text/css"));
    assert_eq!(css.body(), &std::fs::read(fixture("loose-css/loose.css")).unwrap());
}

#[test]
fn responses_allow_cross_origin_use() {
    // Fonts load with CORS; article frames have their own origin.
    let (_dir, library, basic, _) = library();
    for path in [format!("/{basic}/basic.css"), format!("/{basic}/missing.css")] {
        let response = respond(&library, &path);
        assert_eq!(header_of(&response, header::ACCESS_CONTROL_ALLOW_ORIGIN), Some("*"));
    }
}

#[test]
fn path_segments_are_percent_decoded() {
    let (_dir, library, basic, _) = library();
    assert_eq!(respond(&library, &format!("/{basic}/img%2Fdot.png")).status(), StatusCode::OK);
    assert_eq!(respond(&library, &format!("/{basic}/%62asic.css")).status(), StatusCode::OK);
}

#[test]
fn anything_unresolvable_is_an_empty_404() {
    let (_dir, library, basic, loose) = library();
    for path in [
        format!("/{basic}/missing.png"),
        format!("/{basic}/"),
        format!("/{basic}"),
        "/999/basic.css".to_owned(),
        "/abc/basic.css".to_owned(),
        "/".to_owned(),
        format!("/{basic}/%27+replace(_r,"),
        format!("/{loose}/../basic/basic.mdd"),
        format!("/{loose}/..%2Fbasic%2Fbasic.mdd"),
        format!("/{loose}/%2E%2E/basic/basic.mdd"),
    ] {
        let response = respond(&library, &path);
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert!(response.body().is_empty(), "{path}");
    }
}
