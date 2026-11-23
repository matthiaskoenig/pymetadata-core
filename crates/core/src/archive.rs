//! The lazy COMBINE archive, the type the hosts and the writer work with.

use std::collections::HashMap;

use crate::error::OmexError;
use crate::formats::guess_format;
use crate::location::normalize_location;
use crate::manifest::{Entry, Manifest};
use crate::zip::Reader;

/// The location of the archive itself.
const ARCHIVE_LOCATION: &str = ".";
/// The location of the manifest, which the core writes and the caller does not set.
const MANIFEST_LOCATION: &str = "./manifest.xml";

/// Where the bytes of an entry are.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    /// Still in the input, as the member with this index of the reader.
    Member(usize),
    /// Inflated from a member of the input, once, by a caller who asked for it.
    Inflated(Vec<u8>),
    /// Handed over by the caller, replacing what the input holds.
    Replaced(Vec<u8>),
}

/// A COMBINE archive which reads lazily and edits in memory.
///
/// Opening an archive reads the central directory of the zip and inflates the
/// `manifest.xml`, nothing else. A member is inflated when [`Archive::entry_bytes`]
/// asks for it, once, and an entry the caller never replaced stays the compressed member it was
/// read as, which is what the writer copies.
pub struct Archive {
    /// The zip container, which owns the input so that the archive does not borrow from its host.
    reader: Option<Reader>,
    /// The entries in order.
    manifest: Manifest,
    /// The bytes of every entry which has some, by normalized location.
    content: HashMap<String, Content>,
    /// How many members were inflated, which a test reads to see that nothing inflates twice.
    #[cfg(test)]
    inflations: usize,
}

impl Default for Archive {
    /// An empty archive, see [`Archive::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Archive {
    /// Show how many entries there are, and not their bytes.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Archive")
            .field("entries", &self.manifest.entries().len())
            .finish()
    }
}

impl Archive {
    /// An empty archive, holding the two entries the specification requires.
    pub fn new() -> Self {
        Self {
            reader: None,
            manifest: Manifest::default(),
            content: HashMap::new(),
            #[cfg(test)]
            inflations: 0,
        }
    }

    /// Open the archive in `bytes`.
    ///
    /// A zip member the manifest does not list is kept and listed, as the entry its suffix
    /// tells, because real archives look like that. Guessing from the root element would
    /// inflate the member, which nothing does before it is asked for.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the bytes are no zip archive, there is no `manifest.xml` or
    /// it is no manifest, [`OmexError::Refused`] for a member name which points outside of the
    /// archive or holds a character XML forbids, which aborts the open, as does a location of
    /// the manifest which does, and the errors of inflating the manifest, see [`Reader::inflate`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OmexError> {
        Self::from_bytes_with_password(bytes, &[])
    }

    /// Open the password protected archive in `bytes`; an empty password is no password.
    ///
    /// # Errors
    ///
    /// As [`Archive::from_bytes`].
    pub fn from_bytes_with_password(bytes: &[u8], password: &[u8]) -> Result<Self, OmexError> {
        let reader = Reader::new_with_password(bytes.to_vec(), password)?;
        let mut content = HashMap::new();
        let mut manifest_member = None;
        for (index, member) in reader.members().iter().enumerate() {
            // the one place which turns the name of a member into a location
            if member.name.ends_with('/') {
                continue;
            }
            let location = normalize_location(&member.name)?;
            // the archive itself has no bytes, and the manifest is derived state: it is read
            // once here and written from the entries, so neither holds content
            if location == ARCHIVE_LOCATION {
                continue;
            }
            // a later member of the same name wins, as it does for python's `zipfile`
            if location == MANIFEST_LOCATION {
                manifest_member = Some(index);
            } else {
                content.insert(location, Content::Member(index));
            }
        }
        let mut archive = Self {
            reader: Some(reader),
            manifest: Manifest::default(),
            content,
            #[cfg(test)]
            inflations: 0,
        };
        let Some(index) = manifest_member else {
            return Err(OmexError::Malformed(
                "archive has no manifest.xml".to_string(),
            ));
        };
        let xml = archive.inflate_member(index)?;
        let xml = std::str::from_utf8(&xml)
            .map_err(|e| OmexError::Malformed(format!("manifest.xml is not UTF-8: {e}")))?;
        archive.manifest = Manifest::parse(xml)?;
        let mut unlisted: Vec<&String> = archive
            .content
            .keys()
            .filter(|location| archive.manifest.get(location).is_none())
            .collect();
        // a map has no order, the zip has: list the members as the central directory does
        unlisted.sort_by_key(|location| match archive.content.get(*location) {
            Some(Content::Member(index)) => *index,
            _ => usize::MAX,
        });
        let unlisted: Vec<Entry> = unlisted
            .into_iter()
            .map(|location| Entry {
                location: location.clone(),
                format: guess_format(suffix(location), &[]).to_string(),
                master: false,
            })
            .collect();
        for entry in unlisted {
            archive.manifest.add(entry)?;
        }
        Ok(archive)
    }

    /// Change how many uncompressed bytes one member may inflate to.
    ///
    /// The default is [`crate::zip::MAX_INFLATED_SIZE`]; each host has a budget of its own.
    pub fn set_max_inflated_size(&mut self, limit: usize) {
        if let Some(reader) = self.reader.as_mut() {
            reader.set_max_inflated_size(limit);
        }
    }

    /// The manifest: every entry in order.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The locations of the entries, in the order of the manifest.
    pub fn locations(&self) -> impl Iterator<Item = &str> {
        self.manifest
            .entries()
            .iter()
            .map(|entry| entry.location.as_str())
    }

    /// The entry of a location, in any spelling [`normalize_location`] accepts.
    pub fn entry(&self, location: &str) -> Option<&Entry> {
        self.manifest.get(location)
    }

    /// The bytes of an entry, inflating its member the first time and never again.
    ///
    /// # Errors
    ///
    /// [`OmexError::NotFound`] if the entry has no bytes, which holds for `.` and for an entry
    /// the manifest lists but the zip does not hold, [`OmexError::Refused`] for a location
    /// which cannot be normalized and for `./manifest.xml`, which is written from the entries
    /// ([`Archive::manifest`] and [`Manifest::to_xml`] give it), and the errors of
    /// [`Reader::inflate`].
    pub fn entry_bytes(&mut self, location: &str) -> Result<&[u8], OmexError> {
        let key = normalize_location(location)?;
        if key == MANIFEST_LOCATION {
            return Err(OmexError::Refused(format!(
                "the bytes of '{key}' are written from the entries, see manifest() and Manifest::to_xml"
            )));
        }
        if let Some(Content::Member(index)) = self.content.get(&key) {
            let data = self.inflate_member(*index)?;
            self.content.insert(key.clone(), Content::Inflated(data));
        }
        match self.content.get(&key) {
            Some(Content::Inflated(data) | Content::Replaced(data)) => Ok(data),
            _ => Err(OmexError::NotFound(format!(
                "no entry with bytes at '{key}'"
            ))),
        }
    }

    /// Add an entry with its bytes, or replace the entry of the same location.
    ///
    /// The location is normalized. The bytes of `.` and of `./manifest.xml` are not set: the
    /// first has none and the second is written from the manifest.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] for a location outside of the archive, for `.` and
    /// `./manifest.xml`, and for text XML cannot hold; the archive is unchanged then.
    pub fn set_entry(&mut self, entry: Entry, bytes: Vec<u8>) -> Result<(), OmexError> {
        let key = normalize_location(&entry.location)?;
        if key == ARCHIVE_LOCATION || key == MANIFEST_LOCATION {
            return Err(OmexError::Refused(format!(
                "the bytes of '{key}' are not set, the manifest is written from the entries"
            )));
        }
        self.manifest.add(entry)?;
        self.content.insert(key, Content::Replaced(bytes));
        Ok(())
    }

    /// Remove an entry and its bytes, and return the entry.
    ///
    /// `.`, `./manifest.xml` and an unknown location give `None` and change nothing.
    pub fn remove_entry(&mut self, location: &str) -> Option<Entry> {
        let entry = self.manifest.remove(location)?;
        self.content.remove(&entry.location);
        Some(entry)
    }

    /// Inflate the member with this index of the input.
    ///
    /// The one place where a member is inflated.
    ///
    /// A `Content::Member` exists only in an archive which was opened from bytes, and such an
    /// archive always holds its reader, so the error below is an invariant guard and not a path
    /// a caller can reach.
    fn inflate_member(&mut self, index: usize) -> Result<Vec<u8>, OmexError> {
        let reader = self.reader.as_ref().ok_or_else(|| {
            OmexError::Malformed("member of an archive without input".to_string())
        })?;
        let member = reader
            .members()
            .get(index)
            .ok_or_else(|| OmexError::Malformed(format!("no member {index}")))?;
        let data = reader.inflate(member)?;
        #[cfg(test)]
        {
            self.inflations += 1;
        }
        Ok(data)
    }
}

/// The suffix of a location without the dot, empty when the file name has none.
///
/// A leading dot is no suffix, `.hidden` has none, as for python's `Path.suffix`.
fn suffix(location: &str) -> &str {
    let name = location.rsplit('/').next().unwrap_or(location);
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.get(dot + 1..).unwrap_or(""),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An archive of the corpus.
    const SHOWCASE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/omex/CombineArchiveShowCase.omex"
    ));

    /// Opening inflates the manifest and nothing else, and a member inflates once.
    #[test]
    fn entry_bytes_inflates_once() {
        let mut archive = Archive::from_bytes(SHOWCASE).unwrap();
        assert!(archive.content.len() > 2);
        assert_eq!(archive.inflations, 1, "only the manifest");
        let location = archive
            .locations()
            .find(|l| archive.content.contains_key(*l) && *l != MANIFEST_LOCATION)
            .unwrap()
            .to_string();
        let first = archive.entry_bytes(&location).unwrap().to_vec();
        assert_eq!(archive.inflations, 2);
        assert_eq!(archive.entry_bytes(&location).unwrap(), first);
        assert_eq!(archive.inflations, 2, "the second call does not inflate");
    }

    /// The suffix of a location.
    #[test]
    fn suffix_of_a_location() {
        assert_eq!(suffix("./a/b.tar.gz"), "gz");
        assert_eq!(suffix("./a.d/b"), "");
        assert_eq!(suffix("./.hidden"), "");
        assert_eq!(suffix("./a."), "");
    }
}
