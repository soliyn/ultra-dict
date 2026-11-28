//! MDict (`.mdx` + companion `.mdd` volumes).
//!
//! This is the only module that may use `mdictlib`. None of its types,
//! errors or concepts leave this file, so the reader can be replaced (see the
//! licence note in `docs/spec-v1.md`) without touching anything else.
//!
//! Supported through `mdictlib`: MDX 1.2 and 2.0, zlib and LZO blocks,
//! key-index encryption (`Encrypted="2"`). Registration-key encryption
//! (`Encrypted="1"`) is rejected with [`FormatError::NeedsRegistrationKey`].

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use mdictlib::{KeyOrdinal, MddFile, MdxFile};

use super::{Dictionary, EntryLocator, Format, FormatError, Headword, Metadata, display_title};

pub(super) struct Mdict;

impl Format for Mdict {
    fn name(&self) -> &'static str {
        "MDict"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["mdx"]
    }

    fn open(&self, path: &Path) -> Result<Box<dyn Dictionary>, FormatError> {
        let mdx = MdxFile::open(path).map_err(convert_error)?;
        let header = mdx.header();
        let metadata = Metadata {
            title: display_title(header.title(), path),
            description: header
                .description()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_owned),
        };
        Ok(Box::new(MdictDictionary {
            path: path.to_owned(),
            mdx,
            metadata,
            volumes: OnceLock::new(),
        }))
    }
}

struct MdictDictionary {
    path: PathBuf,
    mdx: MdxFile,
    metadata: Metadata,
    /// `.mdd` volumes in search order, opened on the first resource request
    /// so that indexing never pays for them.
    volumes: OnceLock<Vec<MddFile>>,
}

impl MdictDictionary {
    fn volumes(&self) -> Result<&[MddFile], FormatError> {
        if let Some(volumes) = self.volumes.get() {
            return Ok(volumes);
        }
        let opened = mdd_volume_paths(&self.path)
            .iter()
            .map(|path| MddFile::open(path).map_err(convert_error))
            .collect::<Result<Vec<_>, _>>()?;
        // A concurrent caller may have won the race; either result is equal.
        let _ = self.volumes.set(opened);
        Ok(self.volumes.get().expect("volumes were just set"))
    }
}

impl Dictionary for MdictDictionary {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn headword_count(&self) -> u64 {
        self.mdx.len()
    }

    fn headwords(&self) -> Box<dyn Iterator<Item = Result<Headword, FormatError>> + '_> {
        Box::new(self.mdx.keys().map(|row| {
            let row = row.map_err(convert_error)?;
            Ok(Headword {
                locator: EntryLocator::from_raw(row.ordinal().get()),
                text: row.into_key(),
            })
        }))
    }

    fn entry(&self, locator: EntryLocator) -> Result<String, FormatError> {
        let entry = self
            .mdx
            .entry_at(KeyOrdinal::new(locator.to_raw()))
            .map_err(convert_error)?
            .ok_or(FormatError::NoSuchEntry(locator))?;
        let (_, mut text) = entry.into_parts();
        // Many dictionaries NUL-terminate every record.
        let trimmed = text.trim_end_matches('\0').len();
        text.truncate(trimmed);
        Ok(text)
    }

    fn resource(&self, path: &str) -> Result<Option<Vec<u8>>, FormatError> {
        let key = mdd_key(path);
        for volume in self.volumes()? {
            if let Some(resource) = volume.lookup(&key).map_err(convert_error)? {
                let (_, bytes) = resource.into_parts();
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }

    fn redirect_target<'a>(&self, entry: &'a str) -> Option<&'a str> {
        redirect_target(entry)
    }
}

/// The headword an MDict redirect entry (`@@@LINK=target`) points to.
fn redirect_target(entry: &str) -> Option<&str> {
    let target = entry.trim_start().strip_prefix("@@@LINK=")?.trim();
    (!target.is_empty()).then_some(target)
}

/// `dir/name.mdx` → `[dir/name.mdd, dir/name.1.mdd, dir/name.2.mdd, …]`,
/// stopping at the first volume that does not exist.
fn mdd_volume_paths(mdx: &Path) -> Vec<PathBuf> {
    let Some(stem) = mdx.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let dir = mdx.parent().unwrap_or(Path::new(""));
    let mut paths = Vec::new();
    loop {
        let name = match paths.len() {
            0 => format!("{stem}.mdd"),
            n => format!("{stem}.{n}.mdd"),
        };
        let path = dir.join(name);
        if !path.is_file() {
            return paths;
        }
        paths.push(path);
    }
}

/// `img/a.png` or `/img/a.png` → `\img\a.png`, the key form MDD files use.
fn mdd_key(path: &str) -> String {
    let relative = path.trim_start_matches(['/', '\\']);
    format!("\\{}", relative.replace('/', "\\"))
}

fn convert_error(error: mdictlib::Error) -> FormatError {
    use mdictlib::Error as E;
    match error {
        E::Io(e) => FormatError::Io(e),
        E::MissingPasscode | E::InvalidPasscode(_) => FormatError::NeedsRegistrationKey,
        E::Unsupported(what) => FormatError::Unsupported(what.to_owned()),
        other => FormatError::Corrupt(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_entries_name_their_target() {
        assert_eq!(redirect_target("@@@LINK=go
"), Some("go"));
        assert_eq!(redirect_target("@@@LINK=go on"), Some("go on"));
        assert_eq!(redirect_target("@@@LINK=
"), None);
        assert_eq!(redirect_target("<b>go</b> see @@@LINK=went"), None);
    }

    #[test]
    fn resource_paths_become_mdd_keys() {
        assert_eq!(mdd_key("img/a.png"), "\\img\\a.png");
        assert_eq!(mdd_key("/style.css"), "\\style.css");
        assert_eq!(mdd_key("\\style.css"), "\\style.css");
    }
}
