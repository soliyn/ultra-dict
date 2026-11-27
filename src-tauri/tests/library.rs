//! Library tests against the fixtures in `tests/fixtures`, each on a fresh
//! database in a temporary directory.
//!
//! They drive only `ultra_dict_lib::library`'s public interface, so they
//! also hold for any replacement of the format readers or storage behind it.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use ultra_dict_lib::library::{DictionaryInfo, Library, Status};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

/// A library on a new database. Keep the `TempDir` alive for the test.
fn new_library() -> (TempDir, Library) {
    let dir = tempfile::tempdir().expect("temp dir");
    let library = Library::open(&dir.path().join("library.sqlite3")).expect("open library");
    (dir, library)
}

fn add(library: &mut Library, rel: &str) -> DictionaryInfo {
    library
        .add_file(&fixture(rel))
        .unwrap_or_else(|e| panic!("add {rel}: {e}"))
}

#[test]
fn new_library_is_empty() {
    let (_dir, library) = new_library();
    assert_eq!(library.dictionaries().unwrap(), []);
}

#[test]
fn open_creates_missing_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app-data/nested/library.sqlite3");
    Library::open(&path).expect("open");
    assert!(path.is_file());
}

#[test]
fn add_file_indexes_a_dictionary() {
    let (_dir, mut library) = new_library();
    let path = fixture("basic/basic.mdx");
    let info = add(&mut library, "basic/basic.mdx");

    assert_eq!(info.status, Status::Ready);
    assert_eq!(info.path, path);
    assert_eq!(info.title, "Basic Test Dictionary");
    assert_eq!(info.position, 0);
    assert!(info.enabled);
    assert_eq!(info.headword_count, 19, "every entry, duplicates included");

    let meta = fs::metadata(&path).unwrap();
    let file = info.file.expect("size and mtime recorded");
    assert_eq!(file.size, meta.len());
    assert_eq!(file.modified, meta.modified().unwrap());
}

#[test]
fn dictionaries_are_listed_in_the_order_added() {
    let (_dir, mut library) = new_library();
    add(&mut library, "basic/basic.mdx");
    add(&mut library, "encrypted/encrypted.mdx");
    add(&mut library, "untitled/untitled.mdx");

    let listed = library.dictionaries().unwrap();
    let summary: Vec<(u32, &str, bool)> = listed
        .iter()
        .map(|d| (d.position, d.title.as_str(), d.status == Status::Ready))
        .collect();
    assert_eq!(
        summary,
        [
            (0, "Basic Test Dictionary", true),
            (1, "encrypted", false),
            (2, "untitled", true),
        ]
    );
}

#[test]
fn listing_matches_what_add_file_returned() {
    let (_dir, mut library) = new_library();
    let added = vec![
        add(&mut library, "basic/basic.mdx"),
        add(&mut library, "damaged/damaged.mdx"),
    ];
    assert_eq!(library.dictionaries().unwrap(), added);
}

#[test]
fn registration_key_dictionary_reports_the_dedicated_error() {
    let (_dir, mut library) = new_library();
    let info = add(&mut library, "encrypted/encrypted.mdx");
    assert_eq!(
        info.status,
        Status::Error("unsupported: needs registration key".to_owned())
    );
    assert_eq!(info.title, "encrypted", "falls back to the file name");
    assert_eq!(info.headword_count, 0);
    assert!(info.file.is_some(), "the file itself was readable");
}

#[test]
fn unreadable_file_is_recorded_as_error() {
    let (_dir, mut library) = new_library();
    let info = add(&mut library, "basic/no-such-file.mdx");
    let Status::Error(message) = &info.status else {
        panic!("expected an error, got {:?}", info.status);
    };
    assert!(message.starts_with("cannot read file"), "{message}");
    assert_eq!(info.file, None);
    assert_eq!(info.title, "no-such-file");
}

#[test]
fn unsupported_file_type_is_recorded_as_error() {
    let (_dir, mut library) = new_library();
    let info = add(&mut library, "loose-css/loose.css");
    assert!(
        matches!(&info.status, Status::Error(m) if m.starts_with("unsupported")),
        "{:?}",
        info.status
    );
}

#[test]
fn failed_import_leaves_no_headwords_behind() {
    // `damaged.mdx` opens and yields its first headwords, then fails.
    let (_dir, mut library) = new_library();
    let info = add(&mut library, "damaged/damaged.mdx");
    let Status::Error(message) = &info.status else {
        panic!("expected an error, got {:?}", info.status);
    };
    assert!(message.starts_with("damaged dictionary file"), "{message}");
    assert_eq!(info.title, "Damaged Test Dictionary");
    assert_eq!(info.headword_count, 0);

    // The library is still usable afterwards.
    let basic = add(&mut library, "basic/basic.mdx");
    assert_eq!(basic.status, Status::Ready);
    assert_eq!(basic.position, 1);
}

#[test]
fn adding_the_same_path_again_is_a_no_op() {
    let (_dir, mut library) = new_library();
    let first = add(&mut library, "basic/basic.mdx");
    let again = add(&mut library, "basic/basic.mdx");
    assert_eq!(again, first);
    assert_eq!(library.dictionaries().unwrap(), [first]);
}

#[test]
fn library_persists_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    let added = {
        let mut library = Library::open(&db).unwrap();
        vec![
            add(&mut library, "basic/basic.mdx"),
            add(&mut library, "encrypted/encrypted.mdx"),
        ]
    };
    let reopened = Library::open(&db).unwrap();
    assert_eq!(reopened.dictionaries().unwrap(), added);
}
