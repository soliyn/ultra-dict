//! The search key every headword and query is reduced to.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Lowercases `text` and strips diacritics: `"Résumé"` → `"resume"`.
///
/// Lowercasing comes first because it can itself produce combining marks
/// (`'İ'` lowercases to `i` + U+0307).
pub(crate) fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .nfd()
        .filter(|&ch| !is_combining_mark(ch))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn lowercases() {
        assert_eq!(fold("Polish"), "polish");
        assert_eq!(fold("ΣΟΦΙΑ"), "σοφια");
    }

    #[test]
    fn strips_diacritics() {
        assert_eq!(fold("résumé"), "resume");
        assert_eq!(fold("Ångström"), "angstrom");
        assert_eq!(fold("naïve façade"), "naive facade");
        assert_eq!(fold("İstanbul"), "istanbul");
        // Precomposed and decomposed spellings fold alike.
        assert_eq!(fold("e\u{301}"), fold("\u{e9}"));
    }

    #[test]
    fn keeps_everything_else() {
        assert_eq!(fold("rock'n'roll 2.0"), "rock'n'roll 2.0");
        assert_eq!(fold("日本語"), "日本語");
        assert_eq!(fold(""), "");
    }
}
