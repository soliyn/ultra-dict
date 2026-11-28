//! Dictionary file formats.
//!
//! Each supported format is one child module implementing [`Format`] and
//! [`Dictionary`]. Everything outside this module (indexing, search,
//! articles, UI) works only with the types defined here and never names a
//! concrete format or a third-party parser.
//!
//! Adding a format means writing its module and listing it in [`FORMATS`].

mod mdict;

use std::fmt;
use std::path::Path;

/// A compiled-in dictionary format.
pub trait Format: Sync {
    /// Short human-readable name, e.g. `"MDict"`.
    fn name(&self) -> &'static str;

    /// Lowercase file extensions (without the dot) this format opens.
    fn extensions(&self) -> &'static [&'static str];

    /// Opens the dictionary at `path` and reads its metadata.
    ///
    /// Opening must be cheap compared to enumerating headwords: it is also
    /// used to fetch articles and resources from an already indexed file.
    fn open(&self, path: &Path) -> Result<Box<dyn Dictionary>, FormatError>;
}

/// An opened dictionary file.
pub trait Dictionary: Send + Sync {
    /// Title and description.
    fn metadata(&self) -> &Metadata;

    /// Number of headwords [`Dictionary::headwords`] will yield, for
    /// progress reporting. May be an estimate for formats that don't know.
    fn headword_count(&self) -> u64;

    /// Every headword with the locator of its entry, in file order.
    ///
    /// Duplicate headwords are yielded once per entry. After yielding an
    /// error the iterator ends. Dropping the iterator early is how callers
    /// cancel.
    fn headwords(&self) -> Box<dyn Iterator<Item = Result<Headword, FormatError>> + '_>;

    /// The raw article HTML of the entry at `locator`.
    ///
    /// Redirects (MDict `@@@LINK=target`) are returned as-is, not followed:
    /// see [`Dictionary::redirect_target`].
    fn entry(&self, locator: EntryLocator) -> Result<String, FormatError>;

    /// The headword that `entry` (as [`Dictionary::entry`] returned it)
    /// redirects to, if it is a redirect rather than an article. Formats
    /// without redirects keep the default.
    fn redirect_target<'a>(&self, entry: &'a str) -> Option<&'a str> {
        let _ = entry;
        None
    }

    /// The bytes of a resource bundled inside the dictionary's own files,
    /// or `None` if it has none by that name.
    ///
    /// `path` is relative to the dictionary, as written in article HTML,
    /// with `/` separators (`img/a.png`); a leading `/` is ignored. Loose
    /// files next to the dictionary are not this trait's concern.
    fn resource(&self, path: &str) -> Result<Option<Vec<u8>>, FormatError>;
}

/// Every compiled-in format.
static FORMATS: &[&dyn Format] = &[&mdict::Mdict];

/// Opens `path` with the format its extension selects.
pub fn open(path: &Path) -> Result<Box<dyn Dictionary>, FormatError> {
    format_for(path)
        .ok_or_else(|| FormatError::Unsupported("not a supported dictionary file".to_owned()))?
        .open(path)
}

/// Whether some compiled-in format opens files like `path` (by extension).
pub fn is_dictionary_file(path: &Path) -> bool {
    format_for(path).is_some()
}

fn format_for(path: &Path) -> Option<&'static dyn Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    FORMATS
        .iter()
        .copied()
        .find(|format| format.extensions().contains(&ext.as_str()))
}

/// Dictionary metadata shown in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// Display title: plain text, never empty. Falls back to the file name.
    pub title: String,
    /// Description as the dictionary provides it (may contain HTML).
    pub description: Option<String>,
}

/// A headword and where its entry lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headword {
    /// The headword exactly as stored in the dictionary.
    pub text: String,
    /// Where to fetch the entry with [`Dictionary::entry`].
    pub locator: EntryLocator,
}

/// Opaque position of one entry inside one unchanged dictionary file.
///
/// It is the entry's physical position in the file (zero-based, in file
/// order), not anything a particular parser defines, so it stays valid if
/// the reader behind a format is replaced. Store it with
/// [`EntryLocator::to_raw`] and restore it with [`EntryLocator::from_raw`];
/// it is meaningless for any other file or after the file changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryLocator(u64);

impl EntryLocator {
    /// Restores a locator previously produced by [`EntryLocator::to_raw`].
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The value to persist.
    pub const fn to_raw(self) -> u64 {
        self.0
    }
}

/// Why a dictionary file could not be opened or read.
#[derive(Debug)]
pub enum FormatError {
    /// The file could not be read (missing, permissions, …).
    Io(std::io::Error),
    /// The dictionary is locked to a registration key, which is unsupported.
    NeedsRegistrationKey,
    /// The file uses a format or feature this build does not support.
    Unsupported(String),
    /// The file is damaged or not what its extension claims.
    Corrupt(String),
    /// A locator does not name an entry in this file.
    NoSuchEntry(EntryLocator),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read file: {e}"),
            Self::NeedsRegistrationKey => f.write_str("unsupported: needs registration key"),
            Self::Unsupported(what) => write!(f, "unsupported: {what}"),
            Self::Corrupt(what) => write!(f, "damaged dictionary file: {what}"),
            Self::NoSuchEntry(locator) => write!(f, "no entry at position {}", locator.0),
        }
    }
}

impl std::error::Error for FormatError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Builds a display title from a format's raw title: HTML stripped,
/// entities decoded, whitespace collapsed. Falls back to the file stem when
/// the title is absent, blank, or a known authoring-tool placeholder.
pub(crate) fn display_title(raw: Option<&str>, path: &Path) -> String {
    const PLACEHOLDERS: &[&str] = &["Title (No HTML code allowed)"];
    let title = raw.map(strip_html).unwrap_or_default();
    if title.is_empty() || PLACEHOLDERS.iter().any(|p| title.eq_ignore_ascii_case(p)) {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        title
    }
}

/// Removes tags, decodes common entities and collapses whitespace.
fn strip_html(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    decode_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let name = &rest[1..semi];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => {
                    let code = if let Some(hex) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        name.strip_prefix('#').and_then(|dec| dec.parse().ok())
                    };
                    code.and_then(char::from_u32)
                }
            };
            ch.map(|ch| (ch, semi))
        });
        match decoded {
            Some((ch, semi)) => {
                out.push(ch);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(raw: Option<&str>) -> String {
        display_title(raw, Path::new("/dicts/My Dict.mdx"))
    }

    #[test]
    fn title_strips_html_and_entities() {
        assert_eq!(title(Some("<b>Big</b>&nbsp;Dict &amp; Co")), "Big Dict & Co");
        assert_eq!(title(Some("  <font color=red>A</font>\n  B ")), "A B");
        assert_eq!(title(Some("caf&#233; &#x263A;")), "café ☺");
        assert_eq!(title(Some("AT&T")), "AT&T");
    }

    #[test]
    fn title_falls_back_to_file_stem() {
        assert_eq!(title(None), "My Dict");
        assert_eq!(title(Some("")), "My Dict");
        assert_eq!(title(Some("<br/>")), "My Dict");
        assert_eq!(title(Some("Title (No HTML code allowed)")), "My Dict");
    }

    #[test]
    fn formats_are_chosen_by_extension() {
        assert!(is_dictionary_file(Path::new("a.mdx")));
        assert!(is_dictionary_file(Path::new("A.MDX")));
        assert!(!is_dictionary_file(Path::new("a.mdd")));
        assert!(!is_dictionary_file(Path::new("a")));
    }
}
