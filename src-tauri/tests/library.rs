//! Library tests against the fixtures in `tests/fixtures`, each on a fresh
//! database in a temporary directory.
//!
//! They drive only `ultra_dict_lib::library`'s public interface, so they
//! also hold for any replacement of the format readers or storage behind it.

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use tempfile::TempDir;
use ultra_dict_lib::library::{
    Article, ArticleSection, DictionaryId, DictionaryInfo, FolderId, FolderScan, IndexEvent, Library, Progress,
    Resource, Status,
};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

/// A library on a new database. Keep the `TempDir` alive for the test.
fn new_library() -> (TempDir, Library) {
    let dir = tempfile::tempdir().expect("temp dir");
    let library = Library::open(&dir.path().join("library.sqlite3")).expect("open library");
    (dir, library)
}

/// Indexes every dictionary waiting, as the background indexer would.
fn index_all(library: &Library) {
    let mut indexer = library.indexer().expect("indexer");
    while indexer.index_next(|_, _| ControlFlow::Continue(())).expect("index").is_some() {}
}

/// Dictionary `id` as the library lists it now.
fn current(library: &Library, id: DictionaryId) -> DictionaryInfo {
    let listed = library.dictionaries().unwrap();
    listed.into_iter().find(|d| d.id == id).expect("in the library")
}

/// Adds the dictionary file at `path` and indexes it.
fn add_path(library: &mut Library, path: &Path) -> DictionaryInfo {
    let added = library
        .add_file(path)
        .unwrap_or_else(|e| panic!("add {}: {e}", path.display()));
    index_all(library);
    current(library, added.id)
}

/// Adds fixture `rel` and indexes it.
fn add(library: &mut Library, rel: &str) -> DictionaryInfo {
    add_path(library, &fixture(rel))
}

/// Adds the folder at `path` and indexes what it found; `added` is listed
/// as indexed.
fn add_folder(library: &mut Library, path: &Path) -> FolderScan {
    let scan = library
        .add_folder(path)
        .unwrap_or_else(|e| panic!("add folder {}: {e}", path.display()));
    indexed(library, scan)
}

/// Rescans folder `id` and indexes what it found.
fn rescan(library: &mut Library, id: FolderId) -> FolderScan {
    let scan = library.rescan_folder(id).expect("rescan");
    indexed(library, scan)
}

fn indexed(library: &Library, scan: FolderScan) -> FolderScan {
    assert!(scan.added.iter().all(|d| d.status == Status::Indexing), "{:?}", scan.added);
    index_all(library);
    let added = scan.added.iter().map(|d| current(library, d.id)).collect();
    FolderScan { added, ..scan }
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
    assert_eq!(add_path(&mut library, &copy).status, Status::Ready);

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

// --- articles -------------------------------------------------------------

/// The entries of the only section of `headword`'s article.
fn entries(library: &Library, headword: &str) -> Vec<String> {
    let article = library.article(headword).unwrap();
    assert_eq!(article.sections.len(), 1, "{headword:?}: {article:?}");
    article.sections[0].entries.clone().expect("readable")
}

#[test]
fn library_can_be_shared_with_another_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<Library>();
}

#[test]
fn article_returns_the_entry_html() {
    let (_dir, library) = basic_library();
    let article = library.article("go").unwrap();
    let section = &article.sections[0];
    assert_eq!(section.dictionary_title, "Basic Test Dictionary");
    assert_eq!(
        section.dictionary_id,
        library.dictionaries().unwrap()[0].id
    );
    assert_eq!(
        section.entries.as_deref().unwrap(),
        [r#"<div class="entry"><b>go</b> <i>v.</i> (went, gone) to move from one place to another.</div>"#]
    );
}

#[test]
fn article_returns_every_entry_in_file_order() {
    let (_dir, library) = basic_library();
    let banks = entries(&library, "bank");
    assert_eq!(banks.len(), 2);
    assert!(banks[0].contains("<sup>1</sup>"), "{}", banks[0]);
    assert!(banks[1].contains("<sup>2</sup>"), "{}", banks[1]);
}

#[test]
fn article_matches_the_exact_spelling_only() {
    let (_dir, library) = basic_library();
    let polish = entries(&library, "Polish");
    assert_eq!(polish.len(), 1);
    assert!(polish[0].contains("of or from Poland"));
    assert!(entries(&library, "polish")[0].contains("smooth and shiny"));
    assert!(entries(&library, "résumé")[0].contains("summary"));
    assert!(entries(&library, "resume")[0].contains("begin again"));
    assert_eq!(library.article("POLISH").unwrap(), Article { sections: vec![] });
}

#[test]
fn unknown_headword_gives_an_empty_article() {
    let (_dir, library) = basic_library();
    assert_eq!(library.article("nope").unwrap(), Article { sections: vec![] });
    assert_eq!(library.article("").unwrap(), Article { sections: vec![] });
    assert_eq!(library.article("ban").unwrap(), Article { sections: vec![] }, "not a prefix match");
}

// --- redirects ------------------------------------------------------------

/// The one section of `headword`'s article.
fn only_section(library: &Library, headword: &str) -> ArticleSection {
    let mut article = library.article(headword).unwrap();
    assert_eq!(article.sections.len(), 1, "{headword:?}: {article:?}");
    article.sections.remove(0)
}

#[test]
fn a_redirect_is_followed_to_its_target() {
    let (_dir, library) = basic_library();
    let section = only_section(&library, "went");
    assert_eq!(section.redirect.as_deref(), Some("go"));
    assert_eq!(section.entries, Ok(entries(&library, "go")));
}

#[test]
fn an_entry_reached_directly_has_no_redirect() {
    let (_dir, library) = basic_library();
    assert_eq!(only_section(&library, "go").redirect, None);
}

#[test]
fn a_chain_of_five_redirects_is_followed_to_the_end() {
    let (_dir, library) = basic_library();
    let section = only_section(&library, "chain1");
    assert_eq!(section.redirect.as_deref(), Some("chain6"), "the final target");
    assert_eq!(section.entries, Ok(entries(&library, "chain6")));
}

#[test]
fn a_sixth_redirect_in_a_row_is_refused() {
    let (_dir, library) = basic_library();
    let section = only_section(&library, "chain0");
    let error = section.entries.unwrap_err();
    assert!(error.contains("more than 5 redirects"), "{error}");
    assert_eq!(section.redirect, None);
}

#[test]
fn redirect_loops_are_refused() {
    let (_dir, library) = basic_library();
    for (headword, chain) in [("ping", "ping → pong → ping"), ("self", "self → self")] {
        let error = only_section(&library, headword).entries.unwrap_err();
        assert!(error.contains("loop") && error.contains(chain), "{headword}: {error}");
    }
}

#[test]
fn article_opens_the_dictionary_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    add(&mut Library::open(&db).unwrap(), "basic/basic.mdx");
    let reopened = Library::open(&db).unwrap();
    assert!(entries(&reopened, "hello")[0].contains("sound://hello.wav"));
}

#[test]
fn article_sections_follow_library_order() {
    let (dir, mut library) = basic_library();
    let copy = dir.path().join("copy/basic-copy.mdx");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(fixture("basic/basic.mdx"), &copy).unwrap();
    let second = add_path(&mut library, &copy);

    let article = library.article("bank").unwrap();
    let ids: Vec<_> = article.sections.iter().map(|s| s.dictionary_id).collect();
    assert_eq!(ids, [library.dictionaries().unwrap()[0].id, second.id]);
    for section in &article.sections {
        assert_eq!(section.entries.as_ref().unwrap().len(), 2);
    }
}

#[test]
fn unreadable_dictionary_reports_an_error_in_its_section() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    let copy = dir.path().join("copy/basic-copy.mdx");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(fixture("basic/basic.mdx"), &copy).unwrap();
    add_path(&mut Library::open(&db).unwrap(), &copy);
    fs::remove_file(&copy).unwrap();

    let article = Library::open(&db).unwrap().article("go").unwrap();
    assert_eq!(article.sections.len(), 1);
    let error = article.sections[0].entries.as_ref().unwrap_err();
    assert!(error.starts_with("cannot read file"), "{error}");
}

#[test]
fn failed_dictionaries_contribute_no_sections() {
    let (_dir, mut library) = new_library();
    add(&mut library, "damaged/damaged.mdx");
    assert_eq!(library.article("alpha").unwrap(), Article { sections: vec![] });
}

// --- resources ------------------------------------------------------------

fn resource(library: &Library, dict: &DictionaryInfo, path: &str) -> Option<Resource> {
    library
        .resource(dict.id, path)
        .unwrap_or_else(|e| panic!("resource {path:?}: {e}"))
}

fn resource_text(library: &Library, dict: &DictionaryInfo, path: &str) -> Option<String> {
    resource(library, dict, path).map(|r| String::from_utf8(r.bytes).unwrap())
}

/// Copies fixture `rel` (with its `.mdd` volumes) into `dir/sub` and adds it.
fn add_copy(library: &mut Library, dir: &Path, sub: &str, rel: &str) -> DictionaryInfo {
    let from = fixture(rel);
    let folder = dir.join(sub);
    fs::create_dir_all(&folder).unwrap();
    let stem = from.file_stem().unwrap().to_str().unwrap().to_owned();
    for entry in fs::read_dir(from.parent().unwrap()).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_str().unwrap().starts_with(&stem) {
            fs::copy(&path, folder.join(path.file_name().unwrap())).unwrap();
        }
    }
    let info = add_path(library, &folder.join(from.file_name().unwrap()));
    assert_eq!(info.status, Status::Ready);
    info
}

#[test]
fn resources_come_from_the_mdd_with_a_content_type() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");

    let css = resource(&library, &basic, "basic.css").expect("basic.css");
    assert_eq!(css.content_type, "text/css");
    assert!(String::from_utf8(css.bytes).unwrap().contains(".entry"));

    let png = resource(&library, &basic, "img/dot.png").expect("img/dot.png");
    assert_eq!(png.content_type, "image/png");
    assert!(png.bytes.starts_with(b"\x89PNG"));

    let wav = resource(&library, &basic, "hello.wav").expect("hello.wav");
    assert_eq!(wav.content_type, "audio/wav");
}

#[test]
fn resource_paths_accept_leading_and_back_slashes() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    let png = resource(&library, &basic, "img/dot.png").unwrap();
    for path in ["/img/dot.png", r"\img\dot.png", "///img//./dot.png"] {
        assert_eq!(resource(&library, &basic, path).as_ref(), Some(&png), "{path:?}");
    }
}

#[test]
fn loose_files_in_the_folder_are_the_fallback() {
    let (_dir, mut library) = new_library();
    let loose = add(&mut library, "loose-css/loose.mdx");
    let css = resource(&library, &loose, "loose.css").expect("loose.css beside the .mdx");
    assert_eq!(css.content_type, "text/css");
    assert_eq!(css.bytes, fs::read(fixture("loose-css/loose.css")).unwrap());
    assert_eq!(resource(&library, &loose, "/loose.css"), Some(css));
}

#[test]
fn the_mdd_wins_over_a_loose_file_of_the_same_name() {
    let (dir, mut library) = new_library();
    let basic = add_copy(&mut library, dir.path(), "dicts", "basic/basic.mdx");
    fs::write(dir.path().join("dicts/basic.css"), "/* loose */").unwrap();
    fs::create_dir_all(dir.path().join("dicts/js")).unwrap();
    fs::write(dir.path().join("dicts/js/extra.js"), "// loose").unwrap();

    assert!(resource_text(&library, &basic, "basic.css").unwrap().contains(".entry"));
    let js = resource(&library, &basic, "js/extra.js").expect("only loose");
    assert_eq!(js.content_type, "text/javascript");
    assert_eq!(js.bytes, b"// loose");
}

#[test]
fn every_mdd_volume_is_searched_in_volume_order() {
    let (_dir, mut library) = new_library();
    let multi = add(&mut library, "multivolume/multi.mdx");
    assert!(resource(&library, &multi, "multi.css").is_some(), "from multi.mdd");
    assert!(resource(&library, &multi, "img/v1.png").is_some(), "from multi.1.mdd");
    assert_eq!(
        resource_text(&library, &multi, "shared.css").as_deref(),
        Some("/* volume 0 */\n"),
        "the first volume wins"
    );
}

#[test]
fn missing_resources_are_not_found_not_errors() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    let loose = add(&mut library, "loose-css/loose.mdx");
    for path in ["missing.png", "img/", "'+replace(_r,", "basic.css?v=2"] {
        assert_eq!(resource(&library, &basic, path), None, "{path:?}");
    }
    assert_eq!(resource(&library, &loose, "basic.css"), None, "another dictionary's mdd");
}

#[test]
fn resources_of_unknown_or_failed_dictionaries_are_not_found() {
    let (_dir, mut library) = new_library();
    let damaged = add(&mut library, "damaged/damaged.mdx");
    assert_eq!(resource(&library, &damaged, "damaged.mdx"), None);
    assert_eq!(library.resource(DictionaryId::from_raw(999), "basic.css").unwrap(), None);
}

#[test]
fn loose_files_still_resolve_when_the_dictionary_file_is_gone() {
    let (dir, mut library) = new_library();
    let basic = add_copy(&mut library, dir.path(), "dicts", "basic/basic.mdx");
    fs::write(dir.path().join("dicts/extra.css"), "/* loose */").unwrap();
    let db = dir.path().join("library.sqlite3");
    drop(library);
    fs::remove_file(dir.path().join("dicts/basic.mdx")).unwrap();

    let library = Library::open(&db).unwrap();
    assert_eq!(resource(&library, &basic, "basic.css"), None);
    assert_eq!(resource_text(&library, &basic, "extra.css").as_deref(), Some("/* loose */"));
}

#[test]
fn paths_escaping_the_dictionary_folder_are_refused() {
    let (dir, mut library) = new_library();
    let basic = add_copy(&mut library, dir.path(), "dicts/basic", "basic/basic.mdx");
    fs::write(dir.path().join("dicts/secret.txt"), "secret").unwrap();
    fs::write(dir.path().join("dicts/basic/inside.txt"), "inside").unwrap();
    let secret = dir.path().join("dicts/secret.txt");
    let secret = secret.to_str().unwrap();

    for path in [
        "../secret.txt",
        r"..\secret.txt",
        "/../secret.txt",
        "img/../../secret.txt",
        "../basic/inside.txt",
        secret,
        &secret.replace('\\', "/"),
    ] {
        assert_eq!(resource(&library, &basic, path), None, "{path:?}");
    }
    assert!(resource(&library, &basic, "inside.txt").is_some(), "the folder itself is served");
}

// --- allow scripts --------------------------------------------------------

#[test]
fn scripts_are_off_for_new_dictionaries() {
    let (_dir, mut library) = new_library();
    assert!(!add(&mut library, "basic/basic.mdx").allow_scripts);
    assert!(!library.article("go").unwrap().sections[0].allow_scripts);
}

#[test]
fn allow_scripts_is_set_per_dictionary_and_shows_in_articles() {
    let (dir, mut library) = basic_library();
    let first = library.dictionaries().unwrap()[0].clone();
    let copy = dir.path().join("copy/basic-copy.mdx");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(fixture("basic/basic.mdx"), &copy).unwrap();
    let second = add_path(&mut library, &copy);

    let updated = library.set_allow_scripts(first.id, true).unwrap();
    assert_eq!(updated, DictionaryInfo { allow_scripts: true, ..first.clone() });
    let flags: Vec<bool> = library.article("go").unwrap().sections.iter().map(|s| s.allow_scripts).collect();
    assert_eq!(flags, [true, false]);
    assert!(!library.dictionaries().unwrap()[1].allow_scripts, "{second:?}");

    assert!(!library.set_allow_scripts(first.id, false).unwrap().allow_scripts);
}

#[test]
fn allow_scripts_persists_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    {
        let mut library = Library::open(&db).unwrap();
        let id = add(&mut library, "basic/basic.mdx").id;
        library.set_allow_scripts(id, true).unwrap();
    }
    assert!(Library::open(&db).unwrap().dictionaries().unwrap()[0].allow_scripts);
}

#[test]
fn allow_scripts_on_an_unknown_dictionary_is_an_error() {
    let (_dir, mut library) = new_library();
    let error = library.set_allow_scripts(DictionaryId::from_raw(7), true).unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 7 in the library");
}

#[test]
fn an_old_library_is_upgraded_in_place() {
    // The schema as v0.1 created it, with one indexed dictionary.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE dictionaries (
                 id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
                 position INTEGER NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
                 file_size INTEGER, file_mtime_ns INTEGER, status TEXT NOT NULL, error TEXT);
             CREATE TABLE headwords (
                 id INTEGER PRIMARY KEY,
                 dictionary_id INTEGER NOT NULL REFERENCES dictionaries (id) ON DELETE CASCADE,
                 headword TEXT NOT NULL, folded TEXT NOT NULL, locator INTEGER NOT NULL);
             CREATE INDEX headwords_by_folded ON headwords (folded, headword, dictionary_id);
             CREATE INDEX headwords_by_dictionary ON headwords (dictionary_id);
             CREATE TABLE history (
                 id INTEGER PRIMARY KEY, headword TEXT NOT NULL, looked_up_at INTEGER NOT NULL);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dictionaries (path, title, position, status) VALUES (?1, 'Old', 0, 'ready')",
            [fixture("basic/basic.mdx").to_str().unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO headwords (dictionary_id, headword, folded, locator) VALUES (1, 'go', 'go', 0)",
            [],
        )
        .unwrap();
    }

    let mut library = Library::open(&db).unwrap();
    let old = library.dictionaries().unwrap().remove(0);
    assert_eq!((old.title.as_str(), old.allow_scripts), ("Old", false));
    assert_eq!(library.search("g", 0, 10).unwrap().items, ["go"]);
    assert_eq!(old.file_title, "Old");
    assert!(library.set_allow_scripts(old.id, true).unwrap().allow_scripts);
    assert_eq!(library.rename(old.id, "New").unwrap().title, "New");
    let scan = add_folder(&mut library, &fixture("untitled"));
    assert_eq!(scan.added.len(), 1, "folders work after the upgrade");
    drop(library);
    let reopened = Library::open(&db).unwrap().dictionaries().unwrap().remove(0);
    assert!(reopened.allow_scripts, "reopens");
    assert_eq!((reopened.title.as_str(), reopened.file_title.as_str()), ("New", "Old"));

    let mut library = Library::open(&db).unwrap();
    library.reindex(old.id).unwrap();
    index_all(&library);
    let reindexed = current(&library, old.id);
    assert_eq!(reindexed.status, Status::Ready, "reindexing works after the upgrade");
    assert_eq!((reindexed.title.as_str(), reindexed.headword_count), ("New", 19));
}

#[test]
fn dictionary_scripts_are_served_from_the_mdd_and_the_folder() {
    let (_dir, mut library) = new_library();
    let scripts = add(&mut library, "scripts/scripts.mdx");
    let probe = resource(&library, &scripts, "probe.js").expect("probe.js from scripts.mdd");
    assert_eq!(probe.content_type, "text/javascript");
    let loose = resource(&library, &scripts, "loose.js").expect("loose.js beside the .mdx");
    assert_eq!(loose.content_type, "text/javascript");
    assert_eq!(loose.bytes, fs::read(fixture("scripts/loose.js")).unwrap());
}

// --- library management ---------------------------------------------------

/// `basic.mdx` and a copy of it in another folder: two dictionaries with the
/// same headwords.
fn two_basic_dictionaries() -> (TempDir, Library, DictionaryInfo, DictionaryInfo) {
    let (dir, mut library) = new_library();
    let first = add(&mut library, "basic/basic.mdx");
    let copy = dir.path().join("copy/basic-copy.mdx");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(fixture("basic/basic.mdx"), &copy).unwrap();
    let second = add_path(&mut library, &copy);
    (dir, library, first, second)
}

fn section_ids(library: &Library, headword: &str) -> Vec<DictionaryId> {
    library.article(headword).unwrap().sections.iter().map(|s| s.dictionary_id).collect()
}

#[test]
fn disabled_dictionaries_drop_out_of_search_and_articles() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    let untitled = add(&mut library, "untitled/untitled.mdx");

    let updated = library.set_enabled(basic.id, false).unwrap();
    assert_eq!(updated, DictionaryInfo { enabled: false, ..basic.clone() });
    assert_eq!(search_all(&library, "go"), [] as [&str; 0]);
    assert_eq!(search_all(&library, ""), ["nameless"]);
    assert_eq!(library.article("go").unwrap(), Article { sections: vec![] });
    assert_eq!(section_ids(&library, "nameless"), [untitled.id]);

    assert!(library.set_enabled(basic.id, true).unwrap().enabled);
    assert_eq!(search_all(&library, "go"), ["go"]);
    assert_eq!(section_ids(&library, "go"), [basic.id]);
}

#[test]
fn disabling_one_copy_keeps_the_other_section() {
    let (_dir, mut library, first, second) = two_basic_dictionaries();
    library.set_enabled(first.id, false).unwrap();
    assert_eq!(section_ids(&library, "bank"), [second.id]);
    assert_eq!(search_all(&library, "bank"), ["bank"]);
}

fn titles_in_order(library: &Library) -> Vec<(u32, String)> {
    library.dictionaries().unwrap().into_iter().map(|d| (d.position, d.title)).collect()
}

fn three_dictionaries() -> (TempDir, Library, [DictionaryId; 3]) {
    let (dir, mut library) = new_library();
    let ids = [
        add(&mut library, "basic/basic.mdx").id,
        add(&mut library, "encrypted/encrypted.mdx").id,
        add(&mut library, "untitled/untitled.mdx").id,
    ];
    (dir, library, ids)
}

#[test]
fn moving_a_dictionary_reorders_the_library() {
    let (_dir, mut library, [basic, encrypted, untitled]) = three_dictionaries();

    let listed = library.move_dictionary(untitled, 0).unwrap();
    assert_eq!(listed, library.dictionaries().unwrap(), "returns the new listing");
    let order = |l: &Library| l.dictionaries().unwrap().iter().map(|d| (d.position, d.id)).collect::<Vec<_>>();
    assert_eq!(order(&library), [(0, untitled), (1, basic), (2, encrypted)]);

    library.move_dictionary(untitled, 2).unwrap();
    assert_eq!(order(&library), [(0, basic), (1, encrypted), (2, untitled)]);

    library.move_dictionary(basic, 1).unwrap();
    assert_eq!(order(&library), [(0, encrypted), (1, basic), (2, untitled)]);

    // Past the end means last.
    library.move_dictionary(encrypted, 99).unwrap();
    assert_eq!(order(&library), [(0, basic), (1, untitled), (2, encrypted)]);
}

#[test]
fn article_sections_follow_a_changed_order() {
    let (_dir, mut library, first, second) = two_basic_dictionaries();
    library.move_dictionary(second.id, 0).unwrap();
    assert_eq!(section_ids(&library, "bank"), [second.id, first.id]);
}

#[test]
fn new_dictionaries_go_last_after_a_reorder() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    add(&mut library, "encrypted/encrypted.mdx");
    library.move_dictionary(basic.id, 1).unwrap();
    assert_eq!(add(&mut library, "untitled/untitled.mdx").position, 2);
    assert_eq!(
        titles_in_order(&library),
        [(0, "encrypted".into()), (1, "Basic Test Dictionary".into()), (2, "untitled".into())]
    );
}

#[test]
fn order_persists_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    {
        let mut library = Library::open(&db).unwrap();
        add(&mut library, "basic/basic.mdx");
        let untitled = add(&mut library, "untitled/untitled.mdx");
        library.move_dictionary(untitled.id, 0).unwrap();
    }
    assert_eq!(
        titles_in_order(&Library::open(&db).unwrap()),
        [(0, "untitled".into()), (1, "Basic Test Dictionary".into())]
    );
}

#[test]
fn moving_an_unknown_dictionary_is_an_error() {
    let (_dir, mut library, _) = three_dictionaries();
    let error = library.move_dictionary(DictionaryId::from_raw(99), 0).unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 99 in the library");
}

#[test]
fn renaming_changes_the_display_name_everywhere() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    assert_eq!(basic.file_title, "Basic Test Dictionary");

    let renamed = library.rename(basic.id, "  Basic  ").unwrap();
    assert_eq!(
        renamed,
        DictionaryInfo { title: "Basic".into(), ..basic.clone() },
        "trimmed; the file's own title is kept"
    );
    assert_eq!(library.dictionaries().unwrap(), [renamed]);
    assert_eq!(library.article("go").unwrap().sections[0].dictionary_title, "Basic");
}

#[test]
fn an_empty_name_restores_the_file_title() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    library.rename(basic.id, "Basic").unwrap();
    assert_eq!(library.rename(basic.id, " ").unwrap(), basic);
    library.rename(basic.id, "Basic").unwrap();
    assert_eq!(library.rename(basic.id, "").unwrap(), basic);
}

#[test]
fn a_rename_persists_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    let renamed = {
        let mut library = Library::open(&db).unwrap();
        let id = add(&mut library, "encrypted/encrypted.mdx").id;
        library.rename(id, "Locked").unwrap()
    };
    assert_eq!(renamed.title, "Locked", "unusable dictionaries can be renamed too");
    assert_eq!(Library::open(&db).unwrap().dictionaries().unwrap(), [renamed]);
}

#[test]
fn renaming_an_unknown_dictionary_is_an_error() {
    let (_dir, mut library) = new_library();
    let error = library.rename(DictionaryId::from_raw(5), "x").unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 5 in the library");
}

#[test]
fn removing_a_dictionary_takes_it_out_of_everything() {
    let (_dir, mut library, first, second) = two_basic_dictionaries();
    let untitled = add(&mut library, "untitled/untitled.mdx");

    let listed = library.remove(first.id).unwrap();
    assert_eq!(listed, library.dictionaries().unwrap(), "returns the new listing");
    let order: Vec<_> = listed.iter().map(|d| (d.position, d.id)).collect();
    assert_eq!(order, [(0, second.id), (1, untitled.id)], "the rest close up");
    assert_eq!(section_ids(&library, "bank"), [second.id]);
    assert_eq!(library.resource(first.id, "basic.css").unwrap(), None);

    library.remove(second.id).unwrap();
    assert_eq!(search_all(&library, ""), ["nameless"]);
    assert_eq!(library.article("bank").unwrap(), Article { sections: vec![] });
}

#[test]
fn a_removed_dictionary_can_be_added_again() {
    let (_dir, mut library) = new_library();
    let basic = add(&mut library, "basic/basic.mdx");
    library.rename(basic.id, "Basic").unwrap();
    library.remove(basic.id).unwrap();
    assert_eq!(library.dictionaries().unwrap(), []);

    let again = add(&mut library, "basic/basic.mdx");
    assert_eq!((again.title.as_str(), again.position), ("Basic Test Dictionary", 0));
    assert_eq!(search_all(&library, "go"), ["go"]);
    assert_eq!(entries(&library, "bank").len(), 2, "headwords not doubled");
}

#[test]
fn removing_leaves_the_file_alone() {
    let (dir, mut library) = new_library();
    let basic = add_copy(&mut library, dir.path(), "dicts", "basic/basic.mdx");
    library.remove(basic.id).unwrap();
    assert!(dir.path().join("dicts/basic.mdx").is_file());
    assert!(dir.path().join("dicts/basic.mdd").is_file());
}

#[test]
fn removal_persists_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    {
        let mut library = Library::open(&db).unwrap();
        let basic = add(&mut library, "basic/basic.mdx");
        add(&mut library, "untitled/untitled.mdx");
        library.remove(basic.id).unwrap();
    }
    let library = Library::open(&db).unwrap();
    assert_eq!(titles_in_order(&library), [(0, "untitled".into())]);
    assert_eq!(library.search("", 0, 0).unwrap().total_count, 1);
}

#[test]
fn removing_an_unknown_dictionary_is_an_error() {
    let (_dir, mut library) = basic_library();
    let error = library.remove(DictionaryId::from_raw(42)).unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 42 in the library");
    assert_eq!(library.dictionaries().unwrap().len(), 1);
}

#[test]
fn enabled_flag_persists_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    {
        let mut library = Library::open(&db).unwrap();
        let id = add(&mut library, "basic/basic.mdx").id;
        library.set_enabled(id, false).unwrap();
    }
    let library = Library::open(&db).unwrap();
    assert!(!library.dictionaries().unwrap()[0].enabled);
    assert_eq!(search_all(&library, "go"), [] as [&str; 0]);
}

// --- folders --------------------------------------------------------------

/// Copies fixture folder `rel` (every file in it) into `dir/sub`.
fn copy_fixture_folder(dir: &Path, sub: &str, rel: &str) {
    let folder = dir.join(sub);
    fs::create_dir_all(&folder).unwrap();
    for entry in fs::read_dir(fixture(rel)).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, folder.join(path.file_name().unwrap())).unwrap();
    }
}

/// A folder tree of dictionaries under `dir/dicts`:
///
/// ```text
/// dicts/basic/basic.mdx, basic.mdd, notes.txt
/// dicts/more/deeper/untitled.mdx
/// dicts/more/deeper/multi.mdx, multi.mdd, multi.1.mdd
/// ```
fn dictionary_tree(dir: &Path) -> PathBuf {
    copy_fixture_folder(dir, "dicts/basic", "basic");
    fs::write(dir.join("dicts/basic/notes.txt"), "not a dictionary").unwrap();
    copy_fixture_folder(dir, "dicts/more/deeper", "untitled");
    copy_fixture_folder(dir, "dicts/more/deeper", "multivolume");
    dir.join("dicts")
}

fn titles(infos: &[DictionaryInfo]) -> Vec<&str> {
    infos.iter().map(|d| d.title.as_str()).collect()
}

fn file_name(info: &DictionaryInfo) -> &str {
    info.path.file_name().unwrap().to_str().unwrap()
}

#[test]
fn adding_a_folder_adds_every_mdx_in_it_recursively() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());

    let scan = add_folder(&mut library, &folder);
    assert_eq!(
        titles(&scan.added),
        ["Basic Test Dictionary", "Multi-volume Test Dictionary", "untitled"],
        "in path order; .mdd and other files are not dictionaries"
    );
    assert!(scan.added.iter().all(|d| d.status == Status::Ready), "{:?}", scan.added);
    assert_eq!(library.dictionaries().unwrap(), scan.added);
    assert_eq!(search_all(&library, "n"), ["nameless"]);

    // Their resource volumes were found as for a single file.
    let multi = &scan.added[1];
    assert!(resource(&library, multi, "img/v1.png").is_some(), "from multi.1.mdd");
}

#[test]
fn adding_a_folder_remembers_it() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());
    let scan = add_folder(&mut library, &folder);
    assert_eq!(library.folders().unwrap(), std::slice::from_ref(&scan.folder));
    assert_eq!(
        fs::canonicalize(&scan.folder.path).unwrap(),
        fs::canonicalize(&folder).unwrap()
    );

    let db = dir.path().join("library.sqlite3");
    drop(library);
    assert_eq!(Library::open(&db).unwrap().folders().unwrap(), [scan.folder]);
}

#[test]
fn mdx_extension_matches_in_any_case() {
    let (dir, mut library) = new_library();
    fs::create_dir_all(dir.path().join("dicts")).unwrap();
    fs::copy(fixture("untitled/untitled.mdx"), dir.path().join("dicts/LOUD.MDX")).unwrap();
    let scan = add_folder(&mut library, &dir.path().join("dicts"));
    assert_eq!(titles(&scan.added), ["LOUD"]);
}

#[test]
fn unusable_dictionaries_in_a_folder_are_added_with_their_error() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "dicts", "encrypted");
    copy_fixture_folder(dir.path(), "dicts", "untitled");
    let scan = add_folder(&mut library, &dir.path().join("dicts"));
    let statuses: Vec<_> = scan.added.iter().map(|d| (file_name(d), d.status.clone())).collect();
    assert_eq!(
        statuses,
        [
            ("encrypted.mdx", Status::Error("unsupported: needs registration key".into())),
            ("untitled.mdx", Status::Ready),
        ]
    );
}

#[test]
fn an_empty_folder_adds_nothing_but_is_remembered() {
    let (dir, mut library) = new_library();
    fs::create_dir_all(dir.path().join("empty")).unwrap();
    let scan = add_folder(&mut library, &dir.path().join("empty"));
    assert_eq!(scan.added, []);
    assert_eq!(library.folders().unwrap(), [scan.folder]);
}

#[test]
fn adding_a_missing_folder_is_an_error_and_records_nothing() {
    let (dir, mut library) = new_library();
    let error = library.add_folder(&dir.path().join("no-such-folder")).unwrap_err();
    assert!(error.to_string().starts_with("cannot read folder"), "{error}");
    assert_eq!(library.folders().unwrap(), []);
}

#[test]
fn adding_a_file_as_a_folder_is_an_error() {
    let (_dir, mut library) = new_library();
    let error = library.add_folder(&fixture("basic/basic.mdx")).unwrap_err();
    assert!(error.to_string().starts_with("cannot read folder"), "{error}");
    assert_eq!(library.folders().unwrap(), []);
    assert_eq!(library.dictionaries().unwrap(), []);
}

#[test]
fn rescanning_adds_only_dictionaries_placed_there_since() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "dicts/a", "basic");
    let first = add_folder(&mut library, &dir.path().join("dicts"));
    let basic = first.added[0].clone();
    let renamed = library.rename(basic.id, "Mine").unwrap();

    copy_fixture_folder(dir.path(), "dicts/b", "untitled");
    let rescan = rescan(&mut library, first.folder.id);
    assert_eq!(rescan.folder, first.folder);
    assert_eq!(titles(&rescan.added), ["untitled"]);
    assert_eq!(rescan.added[0].position, 1);

    let listed = library.dictionaries().unwrap();
    assert_eq!(listed, [renamed, rescan.added[0].clone()], "the first kept as it was");
    assert_eq!(library.search("", 0, 0).unwrap().total_count, 19, "headwords not doubled");
}

#[test]
fn rescanning_an_unchanged_folder_adds_nothing() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());
    let first = add_folder(&mut library, &folder);
    let before = library.dictionaries().unwrap();
    assert_eq!(rescan(&mut library, first.folder.id).added, []);
    assert_eq!(library.dictionaries().unwrap(), before);
}

#[test]
fn rescan_works_after_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    copy_fixture_folder(dir.path(), "dicts", "basic");
    let folder = add_folder(&mut Library::open(&db).unwrap(), &dir.path().join("dicts")).folder;
    copy_fixture_folder(dir.path(), "dicts", "untitled");
    let scan = rescan(&mut Library::open(&db).unwrap(), folder.id);
    assert_eq!(titles(&scan.added), ["untitled"]);
}

#[test]
fn rescanning_a_folder_that_is_gone_is_an_error_and_keeps_it() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "dicts", "untitled");
    let folder = add_folder(&mut library, &dir.path().join("dicts")).folder;
    fs::remove_dir_all(dir.path().join("dicts")).unwrap();

    let error = library.rescan_folder(folder.id).unwrap_err();
    assert!(error.to_string().starts_with("cannot read folder"), "{error}");
    assert_eq!(library.folders().unwrap(), [folder]);
    assert_eq!(library.dictionaries().unwrap().len(), 1);
}

#[test]
fn rescanning_an_unknown_folder_is_an_error() {
    let (_dir, mut library) = new_library();
    let error = library.rescan_folder(FolderId::from_raw(3)).unwrap_err();
    assert_eq!(error.to_string(), "no folder with id 3 in the library");
}

#[test]
fn adding_the_same_folder_again_is_a_no_op() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());
    let first = add_folder(&mut library, &folder);
    let before = library.dictionaries().unwrap();

    for again in [folder.clone(), folder.join("basic/.."), folder.join(".")] {
        let scan = add_folder(&mut library, &again);
        assert_eq!(scan.folder, first.folder, "{again:?}");
        assert_eq!(scan.added, [], "{again:?}");
    }
    assert_eq!(library.folders().unwrap(), [first.folder]);
    assert_eq!(library.dictionaries().unwrap(), before);
}

#[test]
fn a_folder_skips_dictionaries_already_added_as_files() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());
    let basic = add_path(&mut library, &folder.join("basic/basic.mdx"));

    let scan = add_folder(&mut library, &folder);
    assert_eq!(titles(&scan.added), ["Multi-volume Test Dictionary", "untitled"]);
    let listed = library.dictionaries().unwrap();
    assert_eq!((listed.len(), &listed[0]), (3, &basic));
}

#[test]
fn a_file_already_found_in_a_folder_is_not_added_again() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());
    let scan = add_folder(&mut library, &folder);
    let again = add_path(&mut library, &folder.join("more/deeper/untitled.mdx"));
    assert_eq!(again, scan.added[2]);
    assert_eq!(library.dictionaries().unwrap(), scan.added);
}

#[test]
fn a_nested_folder_adds_nothing_its_parent_already_added() {
    let (dir, mut library) = new_library();
    let folder = dictionary_tree(dir.path());
    add_folder(&mut library, &folder);
    let nested = add_folder(&mut library, &folder.join("more"));
    assert_eq!(nested.added, []);
    assert_eq!(library.folders().unwrap().len(), 2, "both remembered");
}

#[test]
fn the_same_file_spelled_differently_is_not_added_twice() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "dicts/basic", "basic");
    let first = add_path(&mut library, &dir.path().join("dicts/basic/basic.mdx"));
    for spelling in [
        dir.path().join("dicts/basic/./basic.mdx"),
        dir.path().join("dicts/basic/../basic/basic.mdx"),
    ] {
        assert_eq!(add_path(&mut library, &spelling), first, "{spelling:?}");
    }
    assert_eq!(library.dictionaries().unwrap(), [first]);
}

#[cfg(windows)]
#[test]
fn paths_differing_only_in_case_are_the_same_on_windows() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "dicts/basic", "basic");
    let first = add_path(&mut library, &dir.path().join("dicts/basic/basic.mdx"));
    assert_eq!(add_path(&mut library, &dir.path().join("DICTS/BASIC/BASIC.MDX")), first);

    let scan = add_folder(&mut library, &dir.path().join("Dicts"));
    assert_eq!(scan.added, []);
    assert_eq!(add_folder(&mut library, &dir.path().join("dicts")).folder, scan.folder);
    assert_eq!(library.folders().unwrap().len(), 1);
    assert_eq!(library.dictionaries().unwrap(), [first]);
}

#[test]
fn removing_a_folder_forgets_it_and_keeps_its_dictionaries() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "a", "basic");
    copy_fixture_folder(dir.path(), "b", "untitled");
    let a = add_folder(&mut library, &dir.path().join("a"));
    let b = add_folder(&mut library, &dir.path().join("b"));
    let before = library.dictionaries().unwrap();

    assert_eq!(library.remove_folder(a.folder.id).unwrap(), std::slice::from_ref(&b.folder));
    assert_eq!(library.folders().unwrap(), [b.folder]);
    assert_eq!(library.dictionaries().unwrap(), before);
    assert!(dir.path().join("a/basic.mdx").is_file());

    let error = library.remove_folder(a.folder.id).unwrap_err();
    assert_eq!(error.to_string(), format!("no folder with id {} in the library", a.folder.id));
}

#[test]
fn a_removed_dictionary_comes_back_when_its_folder_is_rescanned() {
    let (dir, mut library) = new_library();
    copy_fixture_folder(dir.path(), "dicts", "untitled");
    let scan = add_folder(&mut library, &dir.path().join("dicts"));
    library.remove(scan.added[0].id).unwrap();
    let rescan = rescan(&mut library, scan.folder.id);
    assert_eq!(titles(&rescan.added), ["untitled"]);
}

// --- indexing -------------------------------------------------------------

fn go_on(_: DictionaryId, _: Progress) -> ControlFlow<()> {
    ControlFlow::Continue(())
}

#[test]
fn a_new_dictionary_waits_for_the_indexer() {
    let (_dir, mut library) = new_library();
    let added = library.add_file(&fixture("basic/basic.mdx")).unwrap();
    assert_eq!(added.status, Status::Indexing);
    assert_eq!(added.title, "basic", "the file name until indexed");
    assert_eq!((added.headword_count, added.file), (0, None));
    assert_eq!(library.search("", 0, 10).unwrap().total_count, 0);
    assert_eq!(library.article("go").unwrap(), Article { sections: vec![] });
    assert_eq!(library.resource(added.id, "basic.css").unwrap(), None);

    let mut indexer = library.indexer().unwrap();
    assert_eq!(indexer.index_next(go_on).unwrap(), Some(added.id));
    assert_eq!(indexer.index_next(go_on).unwrap(), None, "nothing else waits");
    let ready = current(&library, added.id);
    assert_eq!(ready.status, Status::Ready);
    assert_eq!(ready.title, "Basic Test Dictionary");
    assert_eq!(search_all(&library, "pol"), ["Polish", "polish"]);
}

#[test]
fn dictionaries_are_indexed_in_the_order_added() {
    let (_dir, mut library) = new_library();
    let untitled = library.add_file(&fixture("untitled/untitled.mdx")).unwrap().id;
    let basic = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    let mut indexer = library.indexer().unwrap();
    assert_eq!(indexer.index_next(go_on).unwrap(), Some(untitled));
    assert_eq!(indexer.index_next(go_on).unwrap(), Some(basic));
    assert_eq!(indexer.index_next(go_on).unwrap(), None);
}

#[test]
fn indexing_reports_progress_after_each_batch() {
    let (_dir, mut library) = new_library();
    let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    let mut seen = Vec::new();
    library
        .indexer()
        .unwrap()
        .batch_size(5)
        .index_next(|dictionary, progress| {
            seen.push((dictionary, progress.done, progress.total));
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(seen, [(id, 0, 19), (id, 5, 19), (id, 10, 19), (id, 15, 19)]);
    assert_eq!(current(&library, id).headword_count, 19);
}

#[test]
fn search_keeps_working_while_another_dictionary_is_indexed() {
    let (_dir, mut library) = basic_library();
    library.add_file(&fixture("untitled/untitled.mdx")).unwrap();
    let mut during = Vec::new();
    library
        .indexer()
        .unwrap()
        .batch_size(1)
        .index_next(|_, _| {
            during.push((search_all(&library, "go"), search_all(&library, "n")));
            ControlFlow::Continue(())
        })
        .unwrap();
    assert!(!during.is_empty());
    for (go, n) in during {
        assert_eq!(go, ["go"], "ready dictionaries stay searchable");
        assert_eq!(n, [] as [&str; 0], "not searchable until indexing completes");
    }
    assert_eq!(search_all(&library, "n"), ["nameless"]);
}

#[test]
fn cancelling_a_run_under_way_keeps_nothing() {
    let (_dir, mut library) = new_library();
    let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    let mut indexer = library.indexer().unwrap().batch_size(5);
    let mut cancelled = None;
    let indexed = indexer
        .index_next(|_, progress| {
            if progress.done == 10 {
                cancelled = Some(library.cancel_indexing(id).unwrap());
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(indexed, Some(id));
    assert_eq!(cancelled.expect("cancelled mid-run").status, Status::NeedsReindex);

    let info = current(&library, id);
    assert_eq!((info.status, info.headword_count), (Status::NeedsReindex, 0));
    assert_eq!(library.search("", 0, 10).unwrap().total_count, 0);
    assert_eq!(library.article("bank").unwrap(), Article { sections: vec![] });
    assert_eq!(indexer.index_next(go_on).unwrap(), None, "not picked up again");

    assert_eq!(library.reindex(id).unwrap().status, Status::Indexing);
    index_all(&library);
    let info = current(&library, id);
    assert_eq!((info.status, info.headword_count), (Status::Ready, 19), "nothing doubled");
    assert_eq!(search_all(&library, ""), BASIC_SORTED);
}

#[test]
fn cancelling_a_waiting_dictionary() {
    let (_dir, mut library) = new_library();
    let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    assert_eq!(library.cancel_indexing(id).unwrap().status, Status::NeedsReindex);
    assert_eq!(library.indexer().unwrap().index_next(go_on).unwrap(), None);
    assert_eq!(library.cancel_indexing(id).unwrap().status, Status::NeedsReindex, "again: no change");

    let ready = add(&mut library, "untitled/untitled.mdx");
    assert_eq!(library.cancel_indexing(ready.id).unwrap(), ready, "nothing to cancel");
    let error = library.cancel_indexing(DictionaryId::from_raw(99)).unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 99 in the library");
}

#[test]
fn an_interrupted_run_needs_reindexing_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.sqlite3");
    let id = {
        let mut library = Library::open(&db).unwrap();
        add(&mut library, "untitled/untitled.mdx");
        let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
        let indexed = library
            .indexer()
            .unwrap()
            .batch_size(5)
            .index_next(|_, progress| {
                if progress.done < 10 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            })
            .unwrap();
        assert_eq!(indexed, Some(id));
        assert_eq!(current(&library, id).status, Status::Indexing, "stopped mid-run");
        assert_eq!(search_all(&library, "bank"), [] as [&str; 0], "half an index is not searchable");
        id
        // The app stops here.
    };

    let mut library = Library::open(&db).unwrap();
    let info = current(&library, id);
    assert_eq!((info.status, info.headword_count), (Status::NeedsReindex, 0));
    assert_eq!(search_all(&library, "bank"), [] as [&str; 0]);
    assert_eq!(search_all(&library, "n"), ["nameless"], "ready dictionaries are untouched");
    assert_eq!(library.indexer().unwrap().index_next(go_on).unwrap(), None, "not resumed by itself");

    library.reindex(id).unwrap();
    index_all(&library);
    let info = current(&library, id);
    assert_eq!((info.status, info.headword_count), (Status::Ready, 19), "nothing doubled");
    assert_eq!(search_all(&library, "bank"), ["bank"]);
}

#[test]
fn reindexing_keeps_the_settings() {
    let (_dir, mut library) = new_library();
    add(&mut library, "untitled/untitled.mdx");
    let id = add(&mut library, "basic/basic.mdx").id;
    library.rename(id, "Mine").unwrap();
    library.set_allow_scripts(id, true).unwrap();
    library.move_dictionary(id, 0).unwrap();
    let before = current(&library, id);

    let reindexing = library.reindex(id).unwrap();
    assert_eq!((reindexing.status, reindexing.headword_count), (Status::Indexing, 0));
    assert_eq!(search_all(&library, "bank"), [] as [&str; 0], "not searchable while reindexing");
    index_all(&library);
    assert_eq!(current(&library, id), before);

    library.set_enabled(id, false).unwrap();
    library.reindex(id).unwrap();
    index_all(&library);
    assert!(!current(&library, id).enabled);

    let error = library.reindex(DictionaryId::from_raw(99)).unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 99 in the library");
}

#[test]
fn reindexing_during_a_run_starts_it_over() {
    let (_dir, mut library) = new_library();
    let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    let mut indexer = library.indexer().unwrap().batch_size(5);
    let mut restarted = false;
    indexer
        .index_next(|_, progress| {
            if progress.done == 10 {
                library.reindex(id).unwrap();
                restarted = true;
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert!(restarted);
    assert_eq!(current(&library, id).status, Status::Indexing, "the new run waits");
    assert_eq!(indexer.index_next(go_on).unwrap(), Some(id));
    let info = current(&library, id);
    assert_eq!((info.status, info.headword_count), (Status::Ready, 19));
    assert_eq!(search_all(&library, ""), BASIC_SORTED);
}

#[test]
fn removing_a_dictionary_during_its_run_stops_it() {
    let (_dir, mut library) = new_library();
    let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    let other = library.add_file(&fixture("untitled/untitled.mdx")).unwrap().id;
    let mut indexer = library.indexer().unwrap().batch_size(5);
    let indexed = indexer
        .index_next(|_, progress| {
            if progress.done == 5 {
                library.remove(id).unwrap();
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(indexed, Some(id));
    assert_eq!(indexer.index_next(go_on).unwrap(), Some(other));
    let ids: Vec<_> = library.dictionaries().unwrap().iter().map(|d| d.id).collect();
    assert_eq!(ids, [other]);
    assert_eq!(search_all(&library, "bank"), [] as [&str; 0]);
    assert_eq!(search_all(&library, "n"), ["nameless"]);
}

#[test]
fn a_dictionary_failing_mid_run_keeps_nothing() {
    let (_dir, mut library) = new_library();
    let id = library.add_file(&fixture("damaged/damaged.mdx")).unwrap().id;
    let mut written = 0;
    library
        .indexer()
        .unwrap()
        .batch_size(1)
        .index_next(|_, progress| {
            written = progress.done;
            ControlFlow::Continue(())
        })
        .unwrap();
    assert!(written > 0, "some headwords were written before the failure");
    let info = current(&library, id);
    assert!(matches!(&info.status, Status::Error(m) if m.starts_with("damaged")), "{:?}", info.status);
    assert_eq!(info.headword_count, 0);
    assert_eq!(search_all(&library, "alpha"), [] as [&str; 0]);
}

#[test]
fn the_background_indexer_indexes_what_it_is_woken_for() {
    let (_dir, mut library) = new_library();
    let (send, events) = mpsc::channel();
    let indexer = library.indexer().unwrap().batch_size(5).spawn(move |event| {
        let _ = send.send(event);
    });
    let wait_until_finished = |id| {
        let mut seen = Vec::new();
        loop {
            let event = events.recv_timeout(Duration::from_secs(30)).expect("an event");
            seen.push(event);
            if event == (IndexEvent::Finished { dictionary: id }) {
                return seen;
            }
        }
    };

    let id = library.add_file(&fixture("basic/basic.mdx")).unwrap().id;
    indexer.wake();
    let progress = |done| IndexEvent::Progress { dictionary: id, progress: Progress { done, total: 19 } };
    assert_eq!(
        wait_until_finished(id),
        [progress(0), progress(5), progress(10), progress(15), IndexEvent::Finished { dictionary: id }]
    );
    assert_eq!(current(&library, id).status, Status::Ready);

    // It sleeps between jobs, and wakes for the next.
    let other = library.add_file(&fixture("untitled/untitled.mdx")).unwrap().id;
    indexer.wake();
    wait_until_finished(other);
    assert_eq!(search_all(&library, "n"), ["nameless"]);
}

// --- startup check and re-locate ------------------------------------------

/// A library on `dir/library.sqlite3` holding `untitled` and, first in
/// order, a copy of `basic` (with its `.mdd`) in `dir/dicts`, renamed
/// "Mine" and with scripts on. Both indexed.
fn library_with_a_copy(dir: &Path) -> (Library, DictionaryInfo) {
    let mut library = Library::open(&dir.join("library.sqlite3")).unwrap();
    add(&mut library, "untitled/untitled.mdx");
    let id = add_copy(&mut library, dir, "dicts", "basic/basic.mdx").id;
    library.rename(id, "Mine").unwrap();
    library.set_allow_scripts(id, true).unwrap();
    library.move_dictionary(id, 0).unwrap();
    let info = current(&library, id);
    (library, info)
}

/// The settings the startup check and re-locating must keep.
fn settings(info: &DictionaryInfo) -> (String, u32, bool, bool) {
    (info.title.clone(), info.position, info.enabled, info.allow_scripts)
}

#[test]
fn the_startup_check_leaves_unchanged_files_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    add(&mut library, "encrypted/encrypted.mdx");
    let before = library.dictionaries().unwrap();

    assert_eq!(library.check_files().unwrap(), []);
    assert_eq!(library.dictionaries().unwrap(), before);
    assert_eq!(current(&library, basic.id).status, Status::Ready);
    assert_eq!(search_all(&library, "bank"), ["bank"]);
}

#[test]
fn a_changed_file_is_reindexed_with_its_settings_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    library.set_enabled(basic.id, false).unwrap();
    let basic = current(&library, basic.id);
    // Another dictionary's content under the same name: size and time change.
    fs::copy(fixture("untitled/untitled.mdx"), &basic.path).unwrap();

    let changed = library.check_files().unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!((changed[0].id, &changed[0].status), (basic.id, &Status::Indexing));
    assert_eq!(settings(&changed[0]), settings(&basic));

    index_all(&library);
    let reindexed = current(&library, basic.id);
    assert_eq!(reindexed.status, Status::Ready);
    assert_eq!(settings(&reindexed), settings(&basic));
    assert_eq!(reindexed.headword_count, 1);
    let meta = fs::metadata(&basic.path).unwrap();
    let file = reindexed.file.unwrap();
    assert_eq!((file.size, file.modified), (meta.len(), meta.modified().unwrap()));
}

#[test]
fn a_changed_file_is_not_searchable_until_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    fs::copy(fixture("untitled/untitled.mdx"), &basic.path).unwrap();

    library.check_files().unwrap();
    assert_eq!(search_all(&library, "bank"), [] as [&str; 0]);
    assert_eq!(search_all(&library, "nameless"), ["nameless"], "only untitled's own copy");
    index_all(&library);
    assert_eq!(search_all(&library, "bank"), [] as [&str; 0]);
    assert_eq!(section_ids(&library, "nameless")[0], basic.id, "the new content, first in order");
}

#[test]
fn a_file_with_only_a_new_modification_time_counts_as_changed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    let file = fs::File::options().write(true).open(&basic.path).unwrap();
    file.set_modified(basic.file.unwrap().modified + Duration::from_secs(60)).unwrap();
    drop(file);

    let changed = library.check_files().unwrap();
    assert_eq!(changed.iter().map(|d| d.id).collect::<Vec<_>>(), [basic.id]);
    index_all(&library);
    assert_eq!(current(&library, basic.id).headword_count, basic.headword_count);
}

#[test]
fn a_missing_file_gets_status_missing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    fs::remove_file(&basic.path).unwrap();

    let changed = library.check_files().unwrap();
    let missing = current(&library, basic.id);
    assert_eq!(changed, std::slice::from_ref(&missing));
    assert_eq!(missing.status, Status::Missing);
    assert_eq!(missing.headword_count, 0);
    assert_eq!(missing.path, basic.path);
    assert_eq!(settings(&missing), settings(&basic));
    assert_eq!(search_all(&library, "bank"), [] as [&str; 0]);
    assert_eq!(section_ids(&library, "go"), []);
    assert_eq!(resource(&library, &missing, "basic.css"), None);

    // Still missing next time: nothing more to change, and it persists.
    assert_eq!(library.check_files().unwrap(), []);
    drop(library);
    let reopened = Library::open(&dir.path().join("library.sqlite3")).unwrap();
    assert_eq!(current(&reopened, basic.id).status, Status::Missing);
}

#[test]
fn a_missing_file_that_is_back_is_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    let aside = dir.path().join("aside.mdx");
    fs::rename(&basic.path, &aside).unwrap();
    library.check_files().unwrap();
    fs::rename(&aside, &basic.path).unwrap();

    let changed = library.check_files().unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].status, Status::Indexing);
    index_all(&library);
    let back = current(&library, basic.id);
    assert_eq!(back.status, Status::Ready);
    assert_eq!(settings(&back), settings(&basic));
    assert_eq!(search_all(&library, "bank"), ["bank"]);
}

#[test]
fn a_failed_dictionary_whose_file_changed_is_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut library = Library::open(&dir.path().join("library.sqlite3")).unwrap();
    let path = dir.path().join("dict.mdx");
    fs::copy(fixture("damaged/damaged.mdx"), &path).unwrap();
    let failed = add_path(&mut library, &path);
    assert!(matches!(failed.status, Status::Error(_)), "{:?}", failed.status);

    fs::copy(fixture("basic/basic.mdx"), &path).unwrap();
    library.check_files().unwrap();
    index_all(&library);
    assert_eq!(current(&library, failed.id).status, Status::Ready);
}

#[test]
fn the_startup_check_leaves_waiting_and_stopped_dictionaries_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    let untitled = library.dictionaries().unwrap()[1].id;
    library.reindex(basic.id).unwrap();
    library.reindex(untitled).unwrap();
    library.cancel_indexing(untitled).unwrap();
    fs::copy(fixture("untitled/untitled.mdx"), &basic.path).unwrap();

    assert_eq!(library.check_files().unwrap(), []);
    assert_eq!(current(&library, basic.id).status, Status::Indexing);
    assert_eq!(current(&library, untitled).status, Status::NeedsReindex);
}

#[test]
fn relocating_a_missing_dictionary_keeps_its_settings() {
    let dir = tempfile::tempdir().unwrap();
    let (library, basic) = library_with_a_copy(dir.path());
    // Moved while the app isn't running (it holds the open files).
    drop(library);
    let moved = dir.path().join("moved");
    fs::rename(dir.path().join("dicts"), &moved).unwrap();
    let mut library = Library::open(&dir.path().join("library.sqlite3")).unwrap();
    library.check_files().unwrap();

    let relocating = library.relocate(basic.id, &moved.join("./basic.mdx")).unwrap();
    assert_eq!(relocating.status, Status::Indexing);
    assert_eq!(relocating.path, dunce::canonicalize(moved.join("basic.mdx")).unwrap());
    index_all(&library);

    let relocated = current(&library, basic.id);
    assert_eq!(relocated.status, Status::Ready);
    assert_eq!(settings(&relocated), settings(&basic));
    assert_eq!(relocated.headword_count, basic.headword_count);
    assert_eq!(search_all(&library, "bank"), ["bank"]);
    assert_eq!(section_ids(&library, "go"), [basic.id]);
    assert!(resource(&library, &relocated, "basic.css").is_some(), "the .mdd beside the new file");
    assert_eq!(library.check_files().unwrap(), [], "the new file is the one on record");
}

#[test]
fn relocating_to_a_file_already_in_the_library_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (mut library, basic) = library_with_a_copy(dir.path());
    fs::remove_file(&basic.path).unwrap();
    library.check_files().unwrap();

    let other = dunce::canonicalize(fixture("untitled/untitled.mdx")).unwrap();
    let error = library.relocate(basic.id, &other).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("{} is already in the library as \"untitled\"", other.display())
    );
    assert_eq!(current(&library, basic.id).status, Status::Missing, "unchanged");
}

#[test]
fn relocating_an_unknown_dictionary_is_an_error() {
    let (_dir, mut library) = new_library();
    let error = library
        .relocate(DictionaryId::from_raw(5), &fixture("basic/basic.mdx"))
        .unwrap_err();
    assert_eq!(error.to_string(), "no dictionary with id 5 in the library");
    assert_eq!(library.dictionaries().unwrap(), []);
}
