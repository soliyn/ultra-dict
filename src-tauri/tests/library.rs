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

// --- search ---------------------------------------------------------------

/// Every distinct headword of `basic.mdx`, in case-insensitive order.
const BASIC_SORTED: &[&str] = &[
    "bank", "chain0", "chain1", "chain2", "chain3", "chain4", "chain5", "chain6", "go", "hello",
    "ping", "Polish", "polish", "pong", "resume", "résumé", "self", "went",
];

fn basic_library() -> (TempDir, Library) {
    let (dir, mut library) = new_library();
    add(&mut library, "basic/basic.mdx");
    (dir, library)
}

/// All results for `query`, in one page.
fn search_all(library: &Library, query: &str) -> Vec<String> {
    let page = library.search(query, 0, 1000).unwrap();
    assert_eq!(page.total_count, page.items.len() as u64, "{query:?}");
    page.items
}

#[test]
fn search_ignores_case_and_diacritics() {
    let (_dir, library) = basic_library();
    assert_eq!(search_all(&library, "resume"), ["resume", "résumé"]);
    assert_eq!(search_all(&library, "RÉSU"), ["resume", "résumé"]);
    assert_eq!(search_all(&library, "Res"), ["resume", "résumé"]);
    assert_eq!(search_all(&library, "e\u{301}"), [] as [&str; 0], "no headword starts with é");
}

#[test]
fn search_is_a_prefix_match() {
    let (_dir, library) = basic_library();
    assert_eq!(search_all(&library, "p"), ["ping", "Polish", "polish", "pong"]);
    assert_eq!(search_all(&library, "chain6"), ["chain6"]);
    assert_eq!(search_all(&library, "ell"), [] as [&str; 0], "no infix matches");
}

#[test]
fn search_keeps_case_variants_as_separate_rows() {
    let (_dir, library) = basic_library();
    assert_eq!(search_all(&library, "pol"), ["Polish", "polish"]);
}

#[test]
fn exact_case_match_sorts_first() {
    let (_dir, library) = basic_library();
    assert_eq!(search_all(&library, "Polish"), ["Polish", "polish"]);
    assert_eq!(search_all(&library, "polish"), ["polish", "Polish"]);
    assert_eq!(search_all(&library, "résumé"), ["résumé", "resume"]);
    // No exact spelling: plain case-insensitive order.
    assert_eq!(search_all(&library, "POLISH"), ["Polish", "polish"]);
}

#[test]
fn search_returns_one_row_per_spelling() {
    // `bank` has two entries in `basic.mdx`; a second copy of the dictionary
    // repeats every headword in another dictionary.
    let (dir, mut library) = basic_library();
    let copy = dir.path().join("copy/basic-copy.mdx");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(fixture("basic/basic.mdx"), &copy).unwrap();
    assert_eq!(library.add_file(&copy).unwrap().status, Status::Ready);

    assert_eq!(search_all(&library, "bank"), ["bank"]);
    assert_eq!(search_all(&library, ""), BASIC_SORTED);
}

#[test]
fn empty_query_pages_from_the_start_of_the_index() {
    let (_dir, library) = basic_library();
    let page = library.search("", 0, 3).unwrap();
    assert_eq!(page.items, ["bank", "chain0", "chain1"]);
    assert_eq!(page.total_count, BASIC_SORTED.len() as u64);
}

#[test]
fn search_pages_concatenate_to_the_full_result() {
    let (_dir, library) = basic_library();
    for query in ["", "p", "polish", "Polish", "résumé", "chain", "go"] {
        let full = search_all(&library, query);
        for page_size in 1..=4 {
            let mut joined = Vec::new();
            for offset in (0..full.len() + page_size).step_by(page_size) {
                let page = library.search(query, offset, page_size).unwrap();
                assert_eq!(page.total_count, full.len() as u64, "{query:?} @{offset}");
                assert!(page.items.len() <= page_size);
                joined.extend(page.items);
            }
            assert_eq!(joined, full, "{query:?} in pages of {page_size}");
        }
    }
}

#[test]
fn search_with_zero_limit_still_counts() {
    let (_dir, library) = basic_library();
    let page = library.search("polish", 0, 0).unwrap();
    assert_eq!(page.items, [] as [&str; 0]);
    assert_eq!(page.total_count, 2);
}

#[test]
fn search_without_matches_is_empty() {
    let (_dir, library) = basic_library();
    let page = library.search("zzz", 0, 50).unwrap();
    assert_eq!(page.items, [] as [&str; 0]);
    assert_eq!(page.total_count, 0);
}

#[test]
fn search_covers_every_ready_dictionary_and_skips_failed_ones() {
    let (_dir, mut library) = new_library();
    add(&mut library, "basic/basic.mdx");
    add(&mut library, "untitled/untitled.mdx");
    add(&mut library, "damaged/damaged.mdx");
    assert_eq!(search_all(&library, "n"), ["nameless"]);
    assert_eq!(search_all(&library, "alpha"), [] as [&str; 0], "damaged is not searchable");
    assert_eq!(library.search("", 0, 0).unwrap().total_count, 19);
}

#[test]
fn search_on_an_empty_library_is_empty() {
    let (_dir, library) = new_library();
    assert_eq!(library.search("", 0, 10).unwrap().total_count, 0);
}
