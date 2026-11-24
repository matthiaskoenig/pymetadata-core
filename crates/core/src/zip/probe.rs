//! Answering whether a container is a COMBINE archive, from its central directory alone.
//!
//! [`crate::Archive::from_bytes`] needs the whole file in memory, which makes a boolean
//! question cost the size of the archive: a host which only asks "is this an archive at all"
//! pays for every member. What answers that question is the central directory, which sits at
//! the end of the file, so a host with a file reads its tail, locates the central directory
//! from the record there, reads those bytes and asks here.
//!
//! Nothing is inflated and nothing is decrypted: an encrypted archive which holds a manifest
//! is an archive, since the question is about the container and not about what may be read
//! from it. A name which points outside of the archive is not refused either, because that
//! refusal belongs to opening an archive and not to recognizing one.

use super::central;
use super::eocd::Eocd;
use super::spec::{EOCD_SIZE, MAX_COMMENT, ZIP64_EOCD_SIZE, ZIP64_LOCATOR_SIZE};
use super::{as_usize, slice_at};
use crate::OmexError;
use crate::location::normalize_location;

/// The location of the manifest, which makes a zip container a COMBINE archive.
const MANIFEST_LOCATION: &str = "./manifest.xml";

/// How many bytes of the end of a file hold every record which says where the central
/// directory is.
///
/// The end-of-central-directory record sits behind an archive comment of up to 64 kB, and the
/// zip64 record and its locator sit in front of it, so this many bytes from the end hold all
/// of them for every container whose comment the format allows.
pub const TAIL_SIZE: usize = MAX_COMMENT + EOCD_SIZE + ZIP64_LOCATOR_SIZE + ZIP64_EOCD_SIZE;

/// Where the central directory of a container is, as offsets of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CentralDirectory {
    /// The offset of the central directory in the file.
    pub offset: u64,
    /// How many bytes it holds.
    pub size: u64,
    /// How many members it lists.
    pub entries: u64,
}

/// Where the central directory is, read from `tail`, the bytes of the file from `base` on.
///
/// `base` is the offset of `tail` in the file, so a host which read the last [`TAIL_SIZE`]
/// bytes passes `file_size - tail.len()`. The offsets of the result are offsets of the file.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the tail holds no end-of-central-directory record, which is
/// what a file that is no zip container looks like, or if the record cannot be reconciled with
/// where it was found. A host which asks whether a file is an archive reads that as "no".
pub fn central_directory(tail: &[u8], base: u64) -> Result<CentralDirectory, OmexError> {
    let eocd = Eocd::find_at(tail, base)?;
    let end = base.saturating_add(tail.len() as u64);
    let size = end.saturating_sub(eocd.cd_offset);
    Ok(CentralDirectory {
        offset: eocd.cd_offset,
        size,
        entries: eocd.entries,
    })
}

/// Whether the central directory in `bytes` declares a member which is the manifest.
///
/// `bytes` are the central directory on its own, as [`CentralDirectory`] locates it, and
/// `entries` is how many members it lists. Every file header is read for its name alone: a
/// name which normalizes to `./manifest.xml` answers yes, a directory member and a name no
/// location can hold are skipped, and nothing else is looked at.
///
/// # Errors
///
/// [`OmexError::Malformed`] if a file header is missing or reaches past the end of `bytes`.
pub fn declares_manifest(bytes: &[u8], entries: u64) -> Result<bool, OmexError> {
    let mut cursor = 0;
    for _ in 0..entries {
        let (member, next) = central::read(bytes, cursor)?;
        if !member.name.ends_with('/')
            && normalize_location(&member.name).is_ok_and(|location| location == MANIFEST_LOCATION)
        {
            return Ok(true);
        }
        cursor = next;
    }
    Ok(false)
}

/// Whether the container in `bytes` declares a manifest, without reading a member.
///
/// This is [`central_directory`] and [`declares_manifest`] over bytes which are the whole
/// container, for a host which has them anyway: it parses the central directory and nothing
/// else, so it costs the size of that directory and not the size of the archive. A host with a
/// file uses the two calls, and reads only the tail and the directory.
///
/// # Errors
///
/// Those of [`central_directory`] and [`declares_manifest`].
pub fn declares_manifest_in(bytes: &[u8]) -> Result<bool, OmexError> {
    let located = central_directory(bytes, 0)?;
    let offset = as_usize(located.offset, "the offset of the central directory")?;
    let size = as_usize(located.size, "the length of the central directory")?;
    let directory = slice_at(bytes, offset, size)?;
    declares_manifest(directory, located.entries)
}
