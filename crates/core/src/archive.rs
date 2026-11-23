//! The lazy COMBINE archive, the type the hosts and the writer work with.

use std::collections::HashMap;

use crate::error::OmexError;
use crate::formats::guess_format;
use crate::location::{normalize_location, suffix};
use crate::manifest::{Entry, Manifest};
use crate::zip::{Member, Reader, Writer};

/// The location of the archive itself.
const ARCHIVE_LOCATION: &str = ".";
/// The location of the manifest, which the core writes and the caller does not set.
const MANIFEST_LOCATION: &str = "./manifest.xml";
/// The compression level the manifest and an entry the caller handed over are deflated at.
///
/// 6 is the default of zlib and of python's `zipfile`, so it is the level the entries of a
/// COMBINE archive carry today. A caller who wants another one writes the whole archive with
/// [`Archive::to_bytes_recompressed`].
const DEFLATE_LEVEL: u32 = 6;

/// Where the bytes of an entry are.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    /// Still in the input, as the member with this index of the reader.
    Member(usize),
    /// Inflated from a member of the input, once, by a caller who asked for it.
    ///
    /// The member it came from is still the one the input holds, so the index stays: an entry
    /// a caller read and did not replace is written as the compressed blob it is, like one
    /// nobody ever asked for.
    Inflated {
        /// The index of the member of the reader the bytes came from.
        index: usize,
        /// The inflated bytes.
        data: Vec<u8>,
    },
    /// Handed over by the caller, replacing what the input holds.
    Replaced(Vec<u8>),
}

/// A member of the source container of an archive, as its central directory describes it.
///
/// See [`Archive::members`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    /// The normalized location of the member, `./` and its name.
    pub location: String,
    /// The number of bytes the member occupies in the container.
    pub compressed_size: u64,
    /// The number of bytes the member holds uncompressed.
    pub uncompressed_size: u64,
    /// The raw zip compression method: 0 stored, 8 deflate.
    pub method: u16,
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
    /// The entries the `manifest.xml` of the container declared, as parsed on opening.
    declared: Vec<Entry>,
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
            declared: Vec::new(),
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
            declared: Vec::new(),
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
        archive.declared = Manifest::parse_declared(xml)?;
        archive.manifest = Manifest::from_declared(archive.declared.clone());
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
    /// The default is [`crate::zip::MAX_INFLATED_SIZE`], which both hosts of this workspace
    /// keep: this is for a host which needs a tighter ceiling, see
    /// [`crate::zip::Reader::with_max_inflated_size`].
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
            let index = *index;
            let data = self.inflate_member(index)?;
            self.content
                .insert(key.clone(), Content::Inflated { index, data });
        }
        match self.content.get(&key) {
            Some(Content::Inflated { data, .. } | Content::Replaced(data)) => Ok(data),
            _ => Err(OmexError::NotFound(format!(
                "no entry with bytes at '{key}'"
            ))),
        }
    }

    /// The entries the `manifest.xml` of the source container declared, exactly as it did.
    ///
    /// One per `content` element in document order. Unlike [`Archive::manifest`] there is no
    /// entry invented for a member the manifest does not list, no required entry prepended
    /// when the document lacks it, and an entry without a member is still there. It is fixed
    /// when the archive is opened: [`Archive::set_entry`] and [`Archive::remove_entry`] do not
    /// change it. A location the manifest declares twice is declared twice here, as parsed,
    /// while [`Archive::manifest`] holds it once, the last declaration winning. Empty for an
    /// archive built with [`Archive::new`].
    pub fn declared_entries(&self) -> &[Entry] {
        &self.declared
    }

    /// The members of the source container, in the order of its central directory.
    ///
    /// This describes the container the archive was opened from and nothing else: it is
    /// empty for an archive built with [`Archive::new`], and it does not grow when
    /// [`Archive::set_entry`] stages content, nor shrink on [`Archive::remove_entry`]. Use
    /// [`Archive::has_bytes`] to ask about the content as it is now. A member the manifest
    /// does not list is in here, `./manifest.xml` is, and a directory member and `.` are not.
    /// A name which occurs twice in the container appears once per member, in zip order, while
    /// the archive holds the bytes of the later one.
    ///
    /// Nothing is inflated and no member bytes are copied: the central directory was parsed
    /// when the archive was opened, and the one allocation is the returned `Vec`.
    pub fn members(&self) -> Vec<MemberInfo> {
        let Some(reader) = self.reader.as_ref() else {
            return Vec::new();
        };
        reader
            .members()
            .iter()
            .filter(|member| !member.name.ends_with('/'))
            .filter_map(|member| {
                let location = normalize_location(&member.name)
                    .expect("from_bytes rejected every member name which does not normalize");
                (location != ARCHIVE_LOCATION).then_some(MemberInfo {
                    location,
                    compressed_size: member.compressed_size,
                    uncompressed_size: member.uncompressed_size,
                    method: member.method,
                })
            })
            .collect()
    }

    /// Whether the location has bytes, in the source container or from [`Archive::set_entry`],
    /// without inflating anything.
    ///
    /// This is `false` for an entry the manifest lists and no member provides, and for `.` and
    /// `./manifest.xml`, which hold no content in this core. It follows [`Archive::remove_entry`]:
    /// a removed entry has no bytes, although [`Archive::members`] still lists its member. A
    /// member the manifest did not list was added to the manifest when the archive was opened,
    /// so removing it is the same.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] for a location which cannot be normalized, as for
    /// [`Archive::entry_bytes`] and [`Archive::set_entry`].
    pub fn has_bytes(&self, location: &str) -> Result<bool, OmexError> {
        let key = normalize_location(location)?;
        Ok(self.content.contains_key(&key))
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

    /// Change the format and the master flag of an entry, leaving its bytes alone.
    ///
    /// [`Archive::set_entry`] takes the bytes of the entry, so changing only its metadata
    /// through it means handing the member back, which inflates it and deflates it again for
    /// a change the member does not carry: the format URI and the master flag live in the
    /// `manifest.xml` and nowhere else. This call changes the manifest entry alone, so a
    /// member of the input stays the compressed blob [`Archive::to_bytes`] copies.
    ///
    /// The entry keeps its position in the manifest. The location is normalized, as it is for
    /// every other call which takes one.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] for a location which cannot be normalized, for `.` and
    /// `./manifest.xml`, whose entries the specification fixes, and for a format holding text
    /// XML cannot carry; [`OmexError::NotFound`] if the manifest has no entry at the location,
    /// since this changes an entry and does not add one. The archive is unchanged then.
    pub fn set_entry_format(
        &mut self,
        location: &str,
        format: &str,
        master: bool,
    ) -> Result<(), OmexError> {
        let key = normalize_location(location)?;
        if key == ARCHIVE_LOCATION || key == MANIFEST_LOCATION {
            return Err(OmexError::Refused(format!(
                "the entry of '{key}' is the one the specification prescribes"
            )));
        }
        if self.manifest.get(&key).is_none() {
            return Err(OmexError::NotFound(format!("no entry at '{key}'")));
        }
        self.manifest.add(Entry::new(&key, format, master)?)
    }

    /// Remove an entry and its bytes, and return the entry.
    ///
    /// The location is normalized, as it is for every other call which takes one. `.`,
    /// `./manifest.xml` and a location the archive does not list give `None` and change nothing:
    /// the two entries the specification requires are not removed.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] for a location outside of the archive and for an absolute one, as
    /// for [`Archive::entry_bytes`] and [`Archive::set_entry`]: a location no archive can hold is
    /// refused here as well, and not answered as the `None` of an archive which does not hold it.
    /// The archive is unchanged then.
    pub fn remove_entry(&mut self, location: &str) -> Result<Option<Entry>, OmexError> {
        let key = normalize_location(location)?;
        let Some(entry) = self.manifest.remove(&key) else {
            return Ok(None);
        };
        self.content.remove(&entry.location);
        Ok(Some(entry))
    }

    /// Reorder the entries this names, among the positions they hold now.
    ///
    /// The write lists the members in the order of the manifest, and the manifest of an
    /// archive which was opened holds the entries its `manifest.xml` declared, then the
    /// members it did not list, in the order of the central directory. A host which keeps its
    /// own order therefore loses it on a write, since nothing it does tells this core what
    /// that order is. This is how it says so.
    ///
    /// The entries of `order` are laid out in the order given, into the slots those same
    /// entries hold today, lowest slot first. An entry `order` does not name stays where it
    /// is, so naming every entry sets the whole order of the written `manifest.xml` and of
    /// the members behind it, while naming a few sorts those few and moves nothing else. See
    /// [`Manifest::reorder`], which this is.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] for a location which cannot be normalized and for one `order`
    /// names twice, [`OmexError::NotFound`] for a location no entry holds; the archive is
    /// unchanged then.
    pub fn reorder_entries(&mut self, order: &[impl AsRef<str>]) -> Result<(), OmexError> {
        self.manifest.reorder(order)
    }

    /// Write the archive, copying every entry which did not change as the blob it is.
    ///
    /// An entry whose bytes the caller never replaced is written as the compressed member it
    /// was read as, with its CRC32, its sizes and its compression method, so that "open an
    /// archive, change one file, write it" deflates the one file and nothing else. This is the
    /// write; [`Archive::to_bytes_recompressed`] is the one which deflates everything again.
    ///
    /// What the written archive is:
    ///
    /// - The members are the entries of the manifest, in its order. `.` is the archive itself
    ///   and is no member, and `./manifest.xml` is written from the entries, so what a caller
    ///   changed in the manifest is in it.
    /// - An entry the manifest lists and the archive holds no bytes for stays a manifest entry
    ///   and gets no member, which is how such an entry was read.
    /// - A directory member of the input is not written: a COMBINE archive lists files, and
    ///   the directories a path needs are made by whoever extracts it.
    /// - An encrypted member the core can decrypt, which is one under the traditional PKWARE
    ///   cipher, is written as the plaintext it holds, deflated. The core writes no encrypted
    ///   archive, as python's `zipfile` writes none, and copying the ciphertext of one member
    ///   would leave an archive half encrypted with a manifest in the clear beside it. Reading
    ///   such an archive needs its password, so writing it needs one too, and an archive which
    ///   was opened without the password is refused.
    /// - An encrypted member the core cannot decrypt is not written at all. A member under
    ///   WinZip AES, compression method 99, has no plaintext the core could write and is no
    ///   blob it may copy either, since the copy would carry the encryption flag into an
    ///   archive whose other members are in the clear, so the write fails as unsupported, as
    ///   reading that member does.
    /// - The bytes in front of an archive are dropped. An archive behind a self extracting stub
    ///   is read, with the offsets of its central directory corrected by the length of the
    ///   stub, and what is written is the archive alone, without the stub.
    /// - A copied member keeps its own modification time and its own external attributes, so
    ///   that reading an archive and writing it back resets neither the timestamp nor the unix
    ///   mode of the files it holds. A member the core compresses itself, which is the manifest
    ///   and every entry the caller handed over, carries the zip epoch, 1980-01-01 00:00:00,
    ///   and a regular file of mode 644: the core has no clock, since `SystemTime::now` is not
    ///   available on `wasm32-unknown-unknown`, and a fixed stamp keeps a write deterministic.
    ///   What a copy does not keep are the extra fields of the member: an Info-ZIP extended
    ///   timestamp (0x5455), which holds the modification time to the second, goes with them,
    ///   and the member keeps the MS-DOS fields, which hold that time rounded down to two
    ///   seconds. See [`Writer::add_raw`] for the whole of it.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if a member is encrypted and the archive was opened without its
    /// password, or if the name of a member is longer than the 16 bit field which holds it;
    /// [`OmexError::Unsupported`] for a member under WinZip AES, and the errors of
    /// [`Reader::inflate`] for a member which has to be read to be written.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>, OmexError> {
        self.write(None)
    }

    /// Write the archive, deflating every entry at `level`.
    ///
    /// This is the write for an archive which was built from scratch or has to come out at a
    /// chosen compression level; [`Archive::to_bytes`] is the one which copies. Every member is
    /// inflated and deflated again, which is what the python implementation did for every
    /// member of every archive on every write, and it costs accordingly.
    ///
    /// The written archive is the one [`Archive::to_bytes`] describes, with every member
    /// deflated at `level`. Nothing is copied here, so no member keeps the modification time it
    /// had: every one of them carries the zip epoch.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if `level` is above 9, the highest level deflate knows, and the
    /// errors of [`Archive::to_bytes`].
    pub fn to_bytes_recompressed(&mut self, level: u32) -> Result<Vec<u8>, OmexError> {
        self.write(Some(level))
    }

    /// Write the archive, deflating every member at `level` when there is one.
    ///
    /// # Errors
    ///
    /// Those of [`Archive::to_bytes`] and [`Archive::to_bytes_recompressed`].
    fn write(&mut self, level: Option<u32>) -> Result<Vec<u8>, OmexError> {
        let recompress = level.is_some();
        let level = level.unwrap_or(DEFLATE_LEVEL);
        // the entries and the manifest are taken first, so that the loop can inflate a member
        // which is not copied and cache it, as a caller who asks for one does
        let entries: Vec<Entry> = self.manifest.entries().to_vec();
        let xml = self.manifest.to_xml();
        let mut writer = Writer::new();
        for entry in &entries {
            let location = entry.location.as_str();
            if location == ARCHIVE_LOCATION {
                continue;
            }
            let name = member_name(location);
            if location == MANIFEST_LOCATION {
                writer.add_deflated(name, xml.as_bytes(), level)?;
                continue;
            }
            match self.action(location, recompress) {
                Action::Nothing => {}
                Action::Copy(index) => {
                    let (reader, member) = self.member(index)?;
                    writer.add_raw(name, member, reader.raw(member)?)?;
                }
                Action::Deflate => {
                    let data = self.entry_bytes(location)?;
                    writer.add_deflated(name, data, level)?;
                }
            }
        }
        Ok(writer.finish())
    }

    /// What the write does with the entry of `location`, which is neither `.` nor the manifest.
    ///
    /// The one place which decides whether a member is copied. A member of the input which the
    /// caller never replaced is, whether or not it was read, unless the whole archive is
    /// deflated again or the member is encrypted, which no member of a written archive is.
    fn action(&self, location: &str, recompress: bool) -> Action {
        let index = match self.content.get(location) {
            None => return Action::Nothing,
            Some(Content::Replaced(_)) => return Action::Deflate,
            Some(Content::Inflated { index, .. } | Content::Member(index)) => *index,
        };
        // a member which is not there is no encrypted one as far as this goes: the copy of
        // `write` is where it is reported
        let encrypted = self
            .member(index)
            .is_ok_and(|(_, member)| member.is_encrypted());
        if recompress || encrypted {
            return Action::Deflate;
        }
        Action::Copy(index)
    }

    /// The input and the member with this index of it.
    ///
    /// A `Content::Member` exists only in an archive which was opened from bytes, and such an
    /// archive always holds its reader, so both errors are invariant guards and not a path a
    /// caller can reach.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the archive has no input or no such member.
    fn member(&self, index: usize) -> Result<(&Reader, &Member), OmexError> {
        let reader = self.reader.as_ref().ok_or_else(|| {
            OmexError::Malformed("member of an archive without input".to_string())
        })?;
        let member = reader
            .members()
            .get(index)
            .ok_or_else(|| OmexError::Malformed(format!("no member {index}")))?;
        Ok((reader, member))
    }

    /// Inflate the member with this index of the input.
    ///
    /// The one place where a member is inflated.
    fn inflate_member(&mut self, index: usize) -> Result<Vec<u8>, OmexError> {
        let (reader, member) = self.member(index)?;
        let data = reader.inflate(member)?;
        #[cfg(test)]
        {
            self.inflations += 1;
        }
        Ok(data)
    }
}

/// What a write does with the bytes of one entry.
enum Action {
    /// Copy the member with this index of the input as the compressed blob it is.
    Copy(usize),
    /// Deflate the bytes of the entry, inflating its member for it when they are not at hand.
    Deflate,
    /// Write no member: the manifest lists the entry and the archive holds no bytes for it.
    Nothing,
}

/// The name of the member which holds the entry of `location`.
///
/// Every location is normalized, so it is `./` and the name of the member behind it. The
/// archive itself (`.`) is no member and is never asked about here.
fn member_name(location: &str) -> &str {
    location.strip_prefix("./").unwrap_or(location)
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
}
