//! The end-of-central-directory record, which says where the central directory is.
//!
//! The record sits at the end of the archive, behind an archive comment of up to 64 kB, so it
//! is found by scanning backwards. A field which does not fit its 16 or 32 bits carries all
//! ones, and the real value is then in the zip64 record the locator in front of it points at.
//!
//! The offsets the record and the central directory give are offsets from the start of the
//! archive, which is not the start of the bytes when something sits in front of it, a self
//! extracting stub or another file the archive was appended to. The record says how long the
//! central directory is, and it was found right behind that central directory, so the
//! difference between where it says the central directory starts and where it actually does is
//! the length of what sits in front, and every offset of the archive is off by it. That is how
//! python's `zipfile` reads such an archive, and the core follows it.

use super::{as_usize, end_of, has_signature, u16_at, u32_at, u64_at};
use crate::OmexError;

/// The signature of an end-of-central-directory record.
const SIGNATURE: [u8; 4] = *b"PK\x05\x06";
/// The size of an end-of-central-directory record, its comment excluded.
const SIZE: usize = 22;
/// The largest archive comment, which bounds the backward scan for the record.
const MAX_COMMENT: usize = 0xffff;
/// The signature of the locator of the zip64 end-of-central-directory record.
const LOCATOR_SIGNATURE: [u8; 4] = *b"PK\x06\x07";
/// The size of that locator, which sits right in front of the record it belongs to.
const LOCATOR_SIZE: usize = 20;
/// The signature of a zip64 end-of-central-directory record.
const ZIP64_SIGNATURE: [u8; 4] = *b"PK\x06\x06";
/// The size of a zip64 end-of-central-directory record without an extensible data sector.
const ZIP64_SIZE: usize = 56;
/// The value a 16 bit field carries when the zip64 record holds the real one.
const SENTINEL16: u16 = 0xffff;
/// The value a 32 bit field carries when the zip64 record holds the real one.
const SENTINEL32: u32 = 0xffff_ffff;

/// Where the central directory of an archive is and how many members it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Eocd {
    /// The number of members the central directory lists.
    pub(super) entries: u64,
    /// The offset of the central directory in the bytes, `prepended` already added.
    pub(super) cd_offset: u64,
    /// The number of bytes in front of the archive, to be added to every offset the central
    /// directory gives. Zero for an ordinary archive.
    pub(super) prepended: u64,
}

/// What a record at the end of an archive declares, 32 bit or zip64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Trailer {
    /// The number of members the central directory lists.
    entries: u64,
    /// The length of the central directory.
    cd_size: u64,
    /// The offset of the central directory from the start of the archive.
    cd_offset: u64,
    /// Where the record was found, which is where the central directory actually ends.
    cd_end: u64,
}

impl Eocd {
    /// Find and read the record at the end of `bytes`.
    ///
    /// The record closest to the end whose comment length matches the bytes behind it wins. If
    /// no record is that consistent, the one closest to the end does, which is the record
    /// python's `zipfile` takes.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the archive holds no such record, if the record or the zip64
    /// record it belongs to reaches past the end of the archive, or if the central directory it
    /// declares cannot be reconciled with where the record was found.
    pub(super) fn find(bytes: &[u8]) -> Result<Eocd, OmexError> {
        let Some(highest) = bytes.len().checked_sub(SIZE) else {
            return Err(OmexError::Malformed(format!(
                "{} bytes are too few for a zip archive",
                bytes.len()
            )));
        };
        let lowest = highest.saturating_sub(MAX_COMMENT);
        let mut last = None;
        for position in (lowest..=highest).rev() {
            if !has_signature(bytes, position, &SIGNATURE) {
                continue;
            }
            let comment = usize::from(u16_at(bytes, position + 20)?);
            if position.saturating_add(SIZE).saturating_add(comment) == bytes.len() {
                return Self::read(bytes, position);
            }
            last = last.or(Some(position));
        }
        match last {
            Some(position) => Self::read(bytes, position),
            None => Err(OmexError::Malformed(
                "the archive has no end-of-central-directory record".to_string(),
            )),
        }
    }

    /// Read the record at `position`, and the zip64 record where a field carries all ones.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if a record reaches past the end of the archive.
    fn read(bytes: &[u8], position: usize) -> Result<Eocd, OmexError> {
        // the scan bounds `position` at 22 bytes before the end, so the fields of the fixed part
        // are inside the buffer and their offsets cannot overflow
        let entries = u16_at(bytes, position + 10)?;
        let cd_size = u32_at(bytes, position + 12)?;
        let cd_offset = u32_at(bytes, position + 16)?;
        // 0xffff members and a 4 GB offset are legal values and not only sentinels, so the
        // 32 bit record stands where no zip64 record backs it
        let sentinel = entries == SENTINEL16 || cd_size == SENTINEL32 || cd_offset == SENTINEL32;
        let trailer = if sentinel {
            Self::zip64(bytes, position)?
        } else {
            None
        };
        Self::locate(trailer.unwrap_or(Trailer {
            entries: entries.into(),
            cd_size: cd_size.into(),
            cd_offset: cd_offset.into(),
            cd_end: position as u64,
        }))
    }

    /// Read the zip64 record which belongs to the record at `position`.
    ///
    /// The locator in front of the record gives the offset of the zip64 record, which is an
    /// offset from the start of the archive and so is off by whatever sits in front of it. The
    /// record sits right in front of its locator, which is where it is looked for first; the
    /// declared offset is the fallback, for a record longer than this one knows.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the locator points past the end of the archive or at
    /// something which is not a zip64 end-of-central-directory record.
    fn zip64(bytes: &[u8], position: usize) -> Result<Option<Trailer>, OmexError> {
        let Some(locator) = position.checked_sub(LOCATOR_SIZE) else {
            return Ok(None);
        };
        if !has_signature(bytes, locator, &LOCATOR_SIGNATURE) {
            return Ok(None);
        }
        let record = match locator.checked_sub(ZIP64_SIZE) {
            Some(behind) if has_signature(bytes, behind, &ZIP64_SIGNATURE) => behind,
            _ => {
                let declared = as_usize(
                    u64_at(bytes, end_of(locator, 8)?)?,
                    "the offset of the zip64 end-of-central-directory record",
                )?;
                if !has_signature(bytes, declared, &ZIP64_SIGNATURE) {
                    return Err(OmexError::Malformed(format!(
                        "no zip64 end-of-central-directory record at offset {declared}"
                    )));
                }
                declared
            }
        };
        Ok(Some(Trailer {
            entries: u64_at(bytes, end_of(record, 32)?)?,
            cd_size: u64_at(bytes, end_of(record, 40)?)?,
            cd_offset: u64_at(bytes, end_of(record, 48)?)?,
            cd_end: record as u64,
        }))
    }

    /// Resolve where the central directory of `trailer` is in the bytes it was found in.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the central directory is longer than the bytes in front of
    /// the record, or if it is declared to start behind where it ends.
    fn locate(trailer: Trailer) -> Result<Eocd, OmexError> {
        let Some(cd_offset) = trailer.cd_end.checked_sub(trailer.cd_size) else {
            return Err(OmexError::Malformed(format!(
                "the central directory of {} bytes does not fit in front of the record at {}",
                trailer.cd_size, trailer.cd_end
            )));
        };
        let Some(prepended) = cd_offset.checked_sub(trailer.cd_offset) else {
            return Err(OmexError::Malformed(format!(
                "the central directory is declared at offset {} but ends at {}, which nothing \
                 in front of the archive can explain",
                trailer.cd_offset, trailer.cd_end
            )));
        };
        Ok(Eocd {
            entries: trailer.entries,
            cd_offset,
            prepended,
        })
    }
}
