//! Smoke test over a local folder of real dictionaries (never committed).
//!
//! ```text
//! ULTRA_DICT_REFERENCE_DIR=/path/to/dictionaries cargo test --release --test reference_set -- --ignored --nocapture
//! ```
//!
//! Every dictionary file found (recursively) is opened through the format
//! layer, all headwords are enumerated, the first and last entries are
//! fetched, and one resource referenced by the first few articles is looked
//! up. Use it to accept a replacement reader against the manual reference set.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ultra_dict_lib::format::{self, Dictionary};

#[test]
#[ignore = "needs ULTRA_DICT_REFERENCE_DIR"]
fn reference_dictionaries_open_and_read() {
    let dir = std::env::var_os("ULTRA_DICT_REFERENCE_DIR").expect("set ULTRA_DICT_REFERENCE_DIR");
    let mut files = Vec::new();
    collect(Path::new(&dir), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no dictionary files found");

    let mut failures = Vec::new();
    for path in &files {
        let started = Instant::now();
        match check(path) {
            Ok(summary) => println!("ok   {} | {summary} | {:?}", path.display(), started.elapsed()),
            Err(e) => {
                println!("FAIL {} | {e}", path.display());
                failures.push(path.clone());
            }
        }
    }
    println!("{} files, {} failed", files.len(), failures.len());
    assert!(failures.is_empty(), "failed: {failures:#?}");
}

fn check(path: &Path) -> Result<String, String> {
    let dict = format::open(path).map_err(|e| e.to_string())?;
    let mut count = 0u64;
    let mut first = None;
    let mut last = None;
    for headword in dict.headwords() {
        let headword = headword.map_err(|e| format!("headword {count}: {e}"))?;
        if first.is_none() {
            first = Some(headword.locator);
        }
        last = Some(headword.locator);
        count += 1;
    }
    if count != dict.headword_count() {
        return Err(format!("{count} headwords, {} declared", dict.headword_count()));
    }
    let mut sampled = Vec::new();
    for locator in [first, last].into_iter().flatten() {
        sampled.push(dict.entry(locator).map_err(|e| format!("entry: {e}"))?);
    }
    let resource = sample_resource(dict.as_ref(), &sampled)?;
    Ok(format!(
        "{:?} | {count} headwords | resource {resource}",
        dict.metadata().title
    ))
}

/// Looks up the first `src="…"` / `href="….css"` reference found in the
/// sampled articles, if any.
fn sample_resource(dict: &dyn Dictionary, articles: &[String]) -> Result<String, String> {
    for article in articles {
        for attr in ["src=\"", "href=\""] {
            for (at, _) in article.match_indices(attr) {
                let rest = &article[at + attr.len()..];
                let Some(end) = rest.find('"') else { continue };
                let target = &rest[..end];
                if target.contains("://") || target.starts_with('#') || target.is_empty() {
                    continue;
                }
                if attr == "href=\"" && !target.ends_with(".css") {
                    continue;
                }
                let found = dict.resource(target).map_err(|e| format!("resource {target}: {e}"))?;
                return Ok(format!("{target:?} {}", if found.is_some() { "in mdd" } else { "not in mdd" }));
            }
        }
    }
    Ok("none referenced".to_owned())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if format::is_dictionary_file(&path) {
            out.push(path);
        }
    }
}
