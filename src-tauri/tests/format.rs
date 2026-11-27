//! Format-layer tests against the fixtures in `tests/fixtures`.
//!
//! They use only `ultra_dict_lib::format`'s own types, never the parser
//! behind it, so they double as the acceptance suite for a replacement
//! MDict reader.

use std::path::{Path, PathBuf};

use ultra_dict_lib::format::{self, Dictionary, EntryLocator, FormatError, Headword};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

fn open(rel: &str) -> Box<dyn Dictionary> {
    format::open(&fixture(rel)).unwrap_or_else(|e| panic!("open {rel}: {e}"))
}

fn headwords(dict: &dyn Dictionary) -> Vec<Headword> {
    dict.headwords().collect::<Result<_, _>>().expect("enumerate headwords")
}

fn entries_for(dict: &dyn Dictionary, word: &str) -> Vec<String> {
    let mut locators: Vec<EntryLocator> = headwords(dict)
        .into_iter()
        .filter(|h| h.text == word)
        .map(|h| h.locator)
        .collect();
    locators.sort();
    locators.into_iter().map(|l| dict.entry(l).unwrap()).collect()
}

fn resource_text(dict: &dyn Dictionary, path: &str) -> Option<String> {
    dict.resource(path)
        .unwrap()
        .map(|bytes| String::from_utf8(bytes).unwrap())
}

#[test]
fn dictionaries_are_shareable_across_threads() {
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn Dictionary>();
}

#[test]
fn metadata_title_is_plain_text() {
    let dict = open("basic/basic.mdx");
    assert_eq!(dict.metadata().title, "Basic Test Dictionary");
    assert_eq!(
        dict.metadata().description.as_deref(),
        Some("Fixture for ultra-dict automated tests.")
    );
}

#[test]
fn metadata_title_falls_back_to_file_name() {
    let dict = open("untitled/untitled.mdx");
    assert_eq!(dict.metadata().title, "untitled");
    assert_eq!(dict.metadata().description, None);
}

#[test]
fn headwords_keep_exact_spelling() {
    let dict = open("basic/basic.mdx");
    let words = headwords(dict.as_ref());
    assert_eq!(words.len() as u64, dict.headword_count());
    for word in ["Polish", "polish", "resume", "résumé", "went", "go", "hello"] {
        assert_eq!(
            words.iter().filter(|h| h.text == word).count(),
            1,
            "{word:?} appears once"
        );
    }
}

#[test]
fn locators_are_distinct_and_survive_persistence() {
    let dict = open("basic/basic.mdx");
    let words = headwords(dict.as_ref());
    let mut raw: Vec<u64> = words.iter().map(|h| h.locator.to_raw()).collect();
    raw.sort();
    raw.dedup();
    assert_eq!(raw.len(), words.len(), "one locator per entry");

    let go = words.iter().find(|h| h.text == "go").unwrap();
    let reopened = open("basic/basic.mdx");
    let restored = EntryLocator::from_raw(go.locator.to_raw());
    assert!(reopened.entry(restored).unwrap().contains("<b>go</b>"));
}

#[test]
fn duplicate_headwords_return_every_entry_in_file_order() {
    let dict = open("basic/basic.mdx");
    let banks = entries_for(dict.as_ref(), "bank");
    assert_eq!(banks.len(), 2);
    assert!(banks[0].contains("<sup>1</sup>"), "{}", banks[0]);
    assert!(banks[1].contains("<sup>2</sup>"), "{}", banks[1]);
}

#[test]
fn redirects_are_returned_unfollowed() {
    let dict = open("basic/basic.mdx");
    assert_eq!(entries_for(dict.as_ref(), "went"), ["@@@LINK=go\r\n"]);
}

#[test]
fn unknown_locator_is_an_error() {
    let dict = open("basic/basic.mdx");
    let past_end = EntryLocator::from_raw(dict.headword_count());
    assert!(matches!(dict.entry(past_end), Err(FormatError::NoSuchEntry(_))));
}

#[test]
fn resources_come_from_the_mdd() {
    let dict = open("basic/basic.mdx");
    assert!(resource_text(dict.as_ref(), "basic.css").unwrap().contains(".entry"));
    let png = dict.resource("img/dot.png").unwrap().unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!(dict.resource("/img/dot.png").unwrap(), Some(png));
    assert!(dict.resource("hello.wav").unwrap().unwrap().starts_with(b"RIFF"));
    assert_eq!(dict.resource("missing.png").unwrap(), None);
}

#[test]
fn mdd_volumes_are_searched_in_order() {
    let dict = open("multivolume/multi.mdx");
    assert!(resource_text(dict.as_ref(), "multi.css").is_some(), "from multi.mdd");
    assert!(dict.resource("img/v1.png").unwrap().is_some(), "from multi.1.mdd");
    assert_eq!(
        resource_text(dict.as_ref(), "shared.css").as_deref(),
        Some("/* volume 0 */\n"),
        "the first volume wins"
    );
}

#[test]
fn dictionary_without_mdd_has_no_bundled_resources() {
    // Loose files beside the .mdx are resolved by the Library, not the format.
    let dict = open("loose-css/loose.mdx");
    assert_eq!(dict.metadata().title, "Loose CSS Test Dictionary");
    assert_eq!(dict.resource("loose.css").unwrap(), None);
}

#[test]
fn registration_key_dictionary_is_rejected() {
    let err = format::open(&fixture("encrypted/encrypted.mdx")).err().unwrap();
    assert!(matches!(err, FormatError::NeedsRegistrationKey), "{err:?}");
    assert_eq!(err.to_string(), "unsupported: needs registration key");
}

#[test]
fn missing_file_is_an_io_error() {
    let err = format::open(&fixture("basic/nope.mdx")).err().unwrap();
    assert!(matches!(err, FormatError::Io(_)), "{err:?}");
}

#[test]
fn unknown_extension_is_unsupported() {
    let err = format::open(&fixture("loose-css/loose.css")).err().unwrap();
    assert!(matches!(err, FormatError::Unsupported(_)), "{err:?}");
}
