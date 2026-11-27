//! Resource paths and content types for [`super::Library::resource`].

use std::path::{Component, Path, PathBuf};

/// A resource path as articles write it, reduced to plain segments.
///
/// Accepts `/` or `\` separators, ignores leading separators, empty and `.`
/// segments. Returns `None` for anything that could leave the dictionary's
/// folder: a `..` segment, a drive prefix (`C:`), or a segment the OS would
/// not read as one plain file name. Leading separators are dropped, so
/// `/etc/x` and `\\server\x` are relative paths inside the folder.
pub(super) fn segments(path: &str) -> Option<Vec<&str>> {
    let segments: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let plain = !segments.is_empty()
        && segments.iter().all(|segment| {
            let mut components = Path::new(segment).components();
            matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
                // `C:` alone is a `Normal` component off Windows; refuse it everywhere.
                && !segment.contains(':')
        });
    plain.then_some(segments)
}

/// `segments` joined under `folder`, if the result is an existing file that
/// really lies inside `folder` (no symlink or junction leading out of it).
pub(super) fn loose_file(folder: &Path, segments: &[&str]) -> Option<PathBuf> {
    let candidate = segments.iter().fold(folder.to_owned(), |path, s| path.join(s));
    let real = candidate.canonicalize().ok()?;
    let inside = real.starts_with(folder.canonicalize().ok()?);
    (inside && real.is_file()).then_some(real)
}

/// The MIME type for a resource, by file extension.
pub(super) fn content_type(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "css" => "text/css",
        "js" => "text/javascript",
        "html" | "htm" => "text/html",
        "txt" => "text/plain",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "spx" => "audio/ogg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_split_into_plain_segments() {
        assert_eq!(segments("img/a.png"), Some(vec!["img", "a.png"]));
        assert_eq!(segments("/img/a.png"), Some(vec!["img", "a.png"]));
        assert_eq!(segments("\\img\\a.png"), Some(vec!["img", "a.png"]));
        assert_eq!(segments("///wavs//./x.wav"), Some(vec!["wavs", "x.wav"]));
        assert_eq!(segments("a b/c%d.css"), Some(vec!["a b", "c%d.css"]));
        // Leading separators never make a path absolute: these stay inside.
        assert_eq!(segments("/etc/passwd"), Some(vec!["etc", "passwd"]));
        assert_eq!(segments(r"\\server\share\x.css"), Some(vec!["server", "share", "x.css"]));
    }

    #[test]
    fn escaping_paths_are_refused() {
        for path in [
            "",
            "/",
            "..",
            "../x.css",
            "img/../../x.css",
            "img/../x.css",
            "..\\x.css",
            "C:/Windows/win.ini",
            "C:\\Windows\\win.ini",
            "c:x.css",
        ] {
            assert_eq!(segments(path), None, "{path:?}");
        }
    }

    #[test]
    fn junk_paths_are_not_errors() {
        // JavaScript fragments some dictionaries put in src/href.
        assert_eq!(segments("'+replace(_r,"), Some(vec!["'+replace(_r,"]));
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type("a/b.CSS"), "text/css");
        assert_eq!(content_type("x.woff2"), "font/woff2");
        assert_eq!(content_type("x.mp3"), "audio/mpeg");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
