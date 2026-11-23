//! The writer of a zip container, which copies a member it is handed as the blob it is.
//!
//! The one number the design rests on: python re-deflates every member of an archive on every
//! write, which costs 234 ms for 1.4 MB, where copying the members which did not change costs
//! 0.12 ms (`spike/RESULTS.md`). [`Writer::add_raw`] is that copy, and it is why the reader
//! hands out the raw bytes of a member at all.
//!
//! The writer holds the archive it builds in a buffer and never touches the filesystem, as
//! everything in the core does: a host hands it bytes and takes bytes back.

use flate2::{Compress, Compression, FlushCompress, Status};

use super::spec::{
    CENTRAL_SIGNATURE, DEFLATE, EOCD_SIGNATURE, LOCAL_SIGNATURE, SENTINEL16_U64, SENTINEL32_U64,
    STORED, UTF8_NAME, VERSION, ZIP64_EOCD_SIGNATURE, ZIP64_EXTRA, ZIP64_LOCATOR_SIGNATURE,
    ZIP64_VERSION,
};
use super::{Member, OmexError, as_usize};

/// The host of the version made by field: 3 is unix, which is what python's `zipfile` writes.
const UNIX: u16 = 3 << 8;
/// The host of the version made by field for a member without a unix mode: 0 is MS-DOS.
///
/// A member whose external attributes carry no mode is written under this host, so that nobody
/// reads the missing mode as a file which may not be read at all. See [`Metadata::host`].
const MS_DOS: u16 = 0;
/// The external attributes of a member the writer compresses itself: a regular file, mode 644.
///
/// The field holds the mode in its high 16 bits for a unix host. Zero would extract as a file
/// without any permission at all, which is why `zipfile` writes a mode here too.
const REGULAR_FILE: u32 = 0o100_644 << 16;
/// The highest compression level deflate knows.
///
/// A level above it is refused here: `flate2` asserts the level of its compressor and would
/// panic, which traps a wasm instance.
const MAX_LEVEL: u32 = 9;
/// The step the buffer of a deflate stream grows by, before it doubles.
const DEFLATE_CHUNK: usize = 64 * 1024;

/// What a member brings along from the archive it came from.
///
/// A member which is copied carries the modification time and the attributes it had, see
/// [`Writer::add_raw`]; a member the writer compresses itself carries [`Metadata::NEW`],
/// because the core has no clock: `SystemTime::now` is not available on
/// `wasm32-unknown-unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Metadata {
    /// The modification time of day in MS-DOS format, two seconds per unit, whose high byte
    /// checks the password of an encrypted member written through a stream.
    time: u16,
    /// The modification date in MS-DOS format: the years since 1980, the month and the day.
    date: u16,
    /// The external file attributes: the unix mode in the high 16 bits, the MS-DOS attribute
    /// byte in the lowest.
    attributes: u32,
}

impl Metadata {
    /// What a member the writer compresses itself carries: the zip epoch and mode 644.
    ///
    /// The zip epoch, 1980-01-01 00:00:00, is what `zipfile.ZipInfo` fills in when it is given
    /// no time either, and the lowest the two fields can hold.
    const NEW: Metadata = Metadata {
        time: 0,
        date: (1 << 5) | 1,
        attributes: REGULAR_FILE,
    };

    /// What a member of another archive brings along.
    fn of(member: &Member) -> Metadata {
        Metadata {
            time: member.dos_time,
            date: member.dos_date,
            attributes: member.external_attributes,
        }
    }

    /// The host of the version made by field, which says how to read the attributes.
    ///
    /// The high half of the attributes is the unix mode, so a member which has one is written
    /// under the unix host, as `zipfile` writes every member of its own. A member without one,
    /// which is what an archive written on Windows or by a java library holds, is written under
    /// MS-DOS: claiming unix for it would say that the file may not be read, written or
    /// executed by anybody, and tools which restore the mode do just that.
    fn host(&self) -> u16 {
        if self.attributes >> 16 == 0 {
            return MS_DOS;
        }
        UNIX
    }
}

/// A zip archive which is built in memory, one member at a time.
///
/// The members are written in the order they are added. Nothing is held back: a member is in
/// the buffer as soon as it was added, and [`Writer::finish`] appends the central directory and
/// the end-of-central-directory record to it.
#[derive(Debug, Default)]
pub struct Writer {
    /// The local file headers and the data of the members, which is the archive but its end.
    body: Vec<u8>,
    /// The central directory, appended to the body by [`Writer::finish`].
    central: Vec<u8>,
    /// The number of members.
    count: u64,
}

impl Writer {
    /// A writer of an empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `data` as a member named `name`, uncompressed.
    ///
    /// The member carries the zip epoch, 1980-01-01 00:00:00, as its modification time, and a
    /// regular file of mode 644 as its external attributes: the core has no clock, and only a
    /// copied member brings a time and a mode of its own.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if the name is longer than the 16 bit field which holds it.
    pub fn add_stored(&mut self, name: &str, data: &[u8]) -> Result<(), OmexError> {
        let crc32 = crc32fast::hash(data);
        self.add(name, STORED, crc32, data, data.len() as u64, Metadata::NEW)
    }

    /// Add `data` as a member named `name`, deflated at `level`.
    ///
    /// The member carries the zip epoch, 1980-01-01 00:00:00, as its modification time, and a
    /// regular file of mode 644 as its external attributes: the core has no clock, and only a
    /// copied member brings a time and a mode of its own.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if the name is longer than the 16 bit field which holds it, or if
    /// `level` is above 9, the highest level deflate knows; [`OmexError::Malformed`] if the
    /// compressor fails on the data.
    pub fn add_deflated(&mut self, name: &str, data: &[u8], level: u32) -> Result<(), OmexError> {
        let deflated = deflate(data, level)?;
        let crc32 = crc32fast::hash(data);
        let size = data.len() as u64;
        self.add(name, DEFLATE, crc32, &deflated, size, Metadata::NEW)
    }

    /// Add the member `member` of another archive under `name`, copying `raw` as it is.
    ///
    /// This is the copy the writer exists for: `raw` are the bytes the other archive stores,
    /// which [`super::Reader::raw`] hands out, and they are written with the compression
    /// method, the CRC32 and the sizes of `member`, so that nothing is inflated and nothing is
    /// deflated again. A method the core cannot inflate is copied too, because the copy leaves
    /// such a member exactly as readable as it was.
    ///
    /// The modification time and the external file attributes of the member are carried over
    /// too, so that an archive which is read and written back resets neither the timestamp nor
    /// the unix mode of the files it holds.
    ///
    /// What is not carried over are the general purpose flags and the extra fields. The member
    /// is written with its sizes in the local file header, so it needs no data descriptor, and
    /// the name says whether it needs the UTF-8 flag; the flags of an encrypted member cannot
    /// be dropped that way, which is why such a member is refused here. The extra fields go
    /// because only the headers which are written here know which of them would still be true:
    /// the one the writer fills in is the zip64 extra field, and it fills it in where its own
    /// sizes and offsets need it. An Info-ZIP extended timestamp (0x5455), which holds the
    /// modification time to the second, is dropped with them, and the member keeps the time of
    /// the MS-DOS fields, which is that time rounded down to two seconds.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if the name is longer than the 16 bit field which holds it, or if
    /// `raw` is not as many bytes as `member` declares; [`OmexError::Unsupported`] if the
    /// member is encrypted, see [`Member::is_encrypted`]; [`OmexError::Malformed`] if the
    /// member is stored and its two sizes differ, which no stored member can do.
    pub fn add_raw(&mut self, name: &str, member: &Member, raw: &[u8]) -> Result<(), OmexError> {
        if member.is_encrypted() {
            return Err(OmexError::Unsupported(format!(
                "member {:?} is encrypted and is not copied as the ciphertext it is: the core \
                 writes no encrypted archive",
                member.name
            )));
        }
        if member.method == STORED && member.compressed_size != member.uncompressed_size {
            return Err(OmexError::Malformed(format!(
                "member {:?} is stored and holds {} bytes for {} uncompressed ones, where a \
                 stored member holds exactly the bytes it uncompresses to",
                member.name, member.compressed_size, member.uncompressed_size
            )));
        }
        if raw.len() as u64 != member.compressed_size {
            return Err(OmexError::Refused(format!(
                "member {:?} holds {} bytes, and {} were handed over to be copied",
                member.name,
                member.compressed_size,
                raw.len()
            )));
        }
        self.add(
            name,
            member.method,
            member.crc32,
            raw,
            member.uncompressed_size,
            Metadata::of(member),
        )
    }

    /// The archive: the members, the central directory and the record which closes it.
    ///
    /// The zip64 records are written in front of the 32 bit record where the archive needs
    /// them, which is at 65535 members, at a central directory of 4 GB, and at a central
    /// directory which starts 4 GB into the archive. Python's `zipfile` writes them from 65536
    /// members and from 2 GB, so an archive between the two thresholds carries a zip64 record
    /// here and none there. The count is the one which has to differ: a count of 65535 is the
    /// sentinel which says that the real count is in the zip64 record, so a reader meets it as
    /// that first, [`super::Reader`] included, and an archive of exactly that many members
    /// needs the record its own count promises. The sizes keep the full 4 GB of their
    /// fields, where `zipfile` stops at 2 GB to stay clear of a reader which holds an offset
    /// in a signed 32 bit integer.
    pub fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.body.len() as u64;
        let cd_size = self.central.len() as u64;
        self.body.append(&mut self.central);
        let zip64 = self.count >= SENTINEL16_U64
            || cd_size >= SENTINEL32_U64
            || cd_offset >= SENTINEL32_U64;
        if zip64 {
            // the zip64 record, then its locator, then the 32 bit record of sentinels: a reader
            // reads the last one, finds the locator right in front of it and follows it back
            let record = self.body.len() as u64;
            self.body.extend_from_slice(&ZIP64_EOCD_SIGNATURE);
            // the length of the record behind this field, i.e. its 56 bytes but the first 12
            self.body.extend_from_slice(&44u64.to_le_bytes());
            self.body
                .extend_from_slice(&(UNIX | ZIP64_VERSION).to_le_bytes());
            self.body.extend_from_slice(&ZIP64_VERSION.to_le_bytes());
            self.body.extend_from_slice(&0u32.to_le_bytes()); // this disk
            self.body.extend_from_slice(&0u32.to_le_bytes()); // the disk of the directory
            self.body.extend_from_slice(&self.count.to_le_bytes());
            self.body.extend_from_slice(&self.count.to_le_bytes());
            self.body.extend_from_slice(&cd_size.to_le_bytes());
            self.body.extend_from_slice(&cd_offset.to_le_bytes());
            self.body.extend_from_slice(&ZIP64_LOCATOR_SIGNATURE);
            self.body.extend_from_slice(&0u32.to_le_bytes()); // the disk of the record
            self.body.extend_from_slice(&record.to_le_bytes());
            self.body.extend_from_slice(&1u32.to_le_bytes()); // the number of disks
        }
        // a field of the 32 bit record which fits carries its value although the zip64 record
        // holds it too, which is how python's `zipfile` writes it; the three are independent
        // here, where the zip64 record holds all of them, and not in a file header, where the
        // extra field holds the two sizes together or neither of them
        let count = narrow16(self.count, self.count >= SENTINEL16_U64);
        self.body.extend_from_slice(&EOCD_SIGNATURE);
        self.body.extend_from_slice(&0u16.to_le_bytes()); // this disk
        self.body.extend_from_slice(&0u16.to_le_bytes()); // the disk of the directory
        self.body.extend_from_slice(&count.to_le_bytes());
        self.body.extend_from_slice(&count.to_le_bytes());
        self.body
            .extend_from_slice(&narrow32(cd_size, cd_size >= SENTINEL32_U64).to_le_bytes());
        self.body
            .extend_from_slice(&narrow32(cd_offset, cd_offset >= SENTINEL32_U64).to_le_bytes());
        self.body.extend_from_slice(&0u16.to_le_bytes()); // no archive comment
        self.body
    }

    /// Write the local file header and the data of a member, and its central directory header.
    ///
    /// `data` is what the archive stores, compressed as `method` says, and `uncompressed_size`
    /// what the member holds inflated; the two are the same for a stored member.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if the name is longer than the 16 bit field which holds it. The
    /// writer is unchanged then: the check runs before the first byte is written.
    fn add(
        &mut self,
        name: &str,
        method: u16,
        crc32: u32,
        data: &[u8],
        uncompressed_size: u64,
        metadata: Metadata,
    ) -> Result<(), OmexError> {
        let record = Record {
            name,
            name_len: name_length(name)?,
            method,
            crc32,
            compressed_size: data.len() as u64,
            uncompressed_size,
            offset: self.body.len() as u64,
            metadata,
        };
        record.write_local(&mut self.body);
        self.body.extend_from_slice(data);
        record.write_central(&mut self.central);
        self.count += 1;
        Ok(())
    }
}

/// One member of the archive, as both of its headers describe it.
struct Record<'a> {
    /// The name of the member.
    name: &'a str,
    /// Its length, which the headers carry in a 16 bit field.
    name_len: u16,
    /// The compression method.
    method: u16,
    /// The CRC32 of the uncompressed bytes.
    crc32: u32,
    /// The number of bytes the member occupies in the archive.
    compressed_size: u64,
    /// The number of bytes the member holds uncompressed.
    uncompressed_size: u64,
    /// The offset of the local file header from the start of the archive.
    offset: u64,
    /// What the member brings along: its modification time and its external attributes.
    metadata: Metadata,
}

impl Record<'_> {
    /// Do the sizes of the member need the zip64 extra field?
    fn sizes64(&self) -> bool {
        self.compressed_size >= SENTINEL32_U64 || self.uncompressed_size >= SENTINEL32_U64
    }

    /// Does the offset of the member need the zip64 extra field?
    fn offset64(&self) -> bool {
        self.offset >= SENTINEL32_U64
    }

    /// The general purpose bit flags of the member.
    ///
    /// A name of only ASCII is the same in UTF-8 and in CP437 and `zipfile` leaves bit 11 off
    /// for it, so an ordinary archive comes out the way `zipfile` writes it. No other bit is
    /// ever set: the sizes are in the headers, so no member carries a data descriptor, and the
    /// core writes no encrypted member.
    fn flags(&self) -> u16 {
        if self.name.is_ascii() { 0 } else { UTF8_NAME }
    }

    /// The version made by field: the host the member comes from and the version it needs.
    ///
    /// The version is the one a reader needs, as `zipfile` writes it in both fields: it is 4.5
    /// for a member which carries the zip64 extra field, so that the two bytes of the central
    /// header do not disagree about the member. See [`Metadata::host`] for the host.
    fn version_made_by(&self) -> u16 {
        self.metadata.host() | self.version()
    }

    /// The version a reader needs to read the member.
    fn version(&self) -> u16 {
        if self.sizes64() || self.offset64() {
            ZIP64_VERSION
        } else {
            VERSION
        }
    }

    /// Append the local file header of the member to `out`.
    ///
    /// The zip64 extra field of a local header holds both sizes, whichever of the two is the
    /// one which does not fit its 32 bit field.
    fn write_local(&self, out: &mut Vec<u8>) {
        let extra = if self.sizes64() {
            zip64_extra(&[self.uncompressed_size, self.compressed_size])
        } else {
            Vec::new()
        };
        out.extend_from_slice(&LOCAL_SIGNATURE);
        out.extend_from_slice(&self.version().to_le_bytes());
        out.extend_from_slice(&self.flags().to_le_bytes());
        out.extend_from_slice(&self.method.to_le_bytes());
        out.extend_from_slice(&self.metadata.time.to_le_bytes());
        out.extend_from_slice(&self.metadata.date.to_le_bytes());
        out.extend_from_slice(&self.crc32.to_le_bytes());
        out.extend_from_slice(&narrow32(self.compressed_size, self.sizes64()).to_le_bytes());
        out.extend_from_slice(&narrow32(self.uncompressed_size, self.sizes64()).to_le_bytes());
        out.extend_from_slice(&self.name_len.to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out.extend_from_slice(&extra);
    }

    /// Append the central directory file header of the member to `out`.
    ///
    /// The zip64 extra field of a central header holds the fields which carry the sentinel, in
    /// the order of the header, which is how a reader tells them apart. The two sizes come as a
    /// pair whenever either of them needs the field, as python's `zipfile` writes them.
    fn write_central(&self, out: &mut Vec<u8>) {
        let mut values = Vec::new();
        if self.sizes64() {
            values.push(self.uncompressed_size);
            values.push(self.compressed_size);
        }
        if self.offset64() {
            values.push(self.offset);
        }
        let extra = zip64_extra(&values);
        out.extend_from_slice(&CENTRAL_SIGNATURE);
        out.extend_from_slice(&self.version_made_by().to_le_bytes());
        out.extend_from_slice(&self.version().to_le_bytes());
        out.extend_from_slice(&self.flags().to_le_bytes());
        out.extend_from_slice(&self.method.to_le_bytes());
        out.extend_from_slice(&self.metadata.time.to_le_bytes());
        out.extend_from_slice(&self.metadata.date.to_le_bytes());
        out.extend_from_slice(&self.crc32.to_le_bytes());
        out.extend_from_slice(&narrow32(self.compressed_size, self.sizes64()).to_le_bytes());
        out.extend_from_slice(&narrow32(self.uncompressed_size, self.sizes64()).to_le_bytes());
        out.extend_from_slice(&self.name_len.to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // no comment
        out.extend_from_slice(&0u16.to_le_bytes()); // the first disk
        out.extend_from_slice(&0u16.to_le_bytes()); // binary content
        out.extend_from_slice(&self.metadata.attributes.to_le_bytes());
        out.extend_from_slice(&narrow32(self.offset, self.offset64()).to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out.extend_from_slice(&extra);
    }
}

/// The length of a member name, for the 16 bit field which holds it.
///
/// # Errors
///
/// [`OmexError::Refused`] if the name is longer than that. A truncated name would be a member
/// under a name which is not the one it was written for.
fn name_length(name: &str) -> Result<u16, OmexError> {
    u16::try_from(name.len()).map_err(|_| {
        OmexError::Refused(format!(
            "the name of a member holds {} bytes, more than the {} the field which holds it \
             can: {name:?}",
            name.len(),
            u16::MAX
        ))
    })
}

/// A zip64 extended information extra field over `values`, empty when there are none.
fn zip64_extra(values: &[u64]) -> Vec<u8> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut extra = Vec::with_capacity(4 + 8 * values.len());
    extra.extend_from_slice(&ZIP64_EXTRA.to_le_bytes());
    extra.extend_from_slice(&((8 * values.len()) as u16).to_le_bytes());
    for value in values {
        extra.extend_from_slice(&value.to_le_bytes());
    }
    extra
}

/// `value` for a 32 bit field, or the sentinel which says that a zip64 record holds it.
///
/// A value which does not fit although `zip64` is false gets the sentinel too, so that a field
/// never holds a value which is not the real one; whether a value fits is what `zip64` says.
fn narrow32(value: u64, zip64: bool) -> u32 {
    if zip64 {
        return u32::MAX;
    }
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// `value` for a 16 bit field, or the sentinel which says that a zip64 record holds it.
fn narrow16(value: u64, zip64: bool) -> u16 {
    if zip64 {
        return u16::MAX;
    }
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Deflate `data` at `level`, as the raw stream a zip member holds.
///
/// This drives `flate2::Compress` and not the writer flate2 offers over the `Write` trait of
/// the standard library: the core uses neither the filesystem nor the stream traits.
///
/// # Errors
///
/// [`OmexError::Refused`] if `level` is above 9, the highest level deflate knows, which the
/// compressor would panic on; [`OmexError::Malformed`] if the compressor fails on the data.
fn deflate(data: &[u8], level: u32) -> Result<Vec<u8>, OmexError> {
    if level > MAX_LEVEL {
        return Err(OmexError::Refused(format!(
            "the compression level is {level}, and deflate knows 0 to {MAX_LEVEL}"
        )));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut state = Compress::new(Compression::new(level), false);
    loop {
        if out.len() == out.capacity() {
            // the buffer doubles, as a vector does, so that no realloc copies every step
            out.reserve(out.capacity().max(DEFLATE_CHUNK));
        }
        let read = state.total_in();
        let written = state.total_out();
        let consumed = as_usize(read, "the length of the deflated bytes")?;
        let rest = data.get(consumed..).ok_or_else(|| {
            OmexError::Malformed(format!(
                "the compressor consumed {consumed} of the {} bytes of a member",
                data.len()
            ))
        })?;
        let status = state
            .compress_vec(rest, &mut out, FlushCompress::Finish)
            .map_err(|error| OmexError::Malformed(format!("a member does not deflate: {error}")))?;
        if status == Status::StreamEnd {
            return Ok(out);
        }
        if state.total_in() == read && state.total_out() == written && out.len() < out.capacity() {
            // the compressor neither read nor wrote although it had room for both, which no
            // deflate stream does: it is an error and not a loop which never ends
            return Err(OmexError::Malformed(
                "the compressor stopped before the end of a member".to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::eocd::Eocd;
    use super::super::{Reader, central, local};
    use super::*;

    /// Read the members of an archive this module wrote, with the reader of the core.
    fn members(bytes: &[u8]) -> Vec<Member> {
        let eocd = Eocd::find(bytes).unwrap();
        central::members(bytes, &eocd).unwrap()
    }

    /// The version the first central directory header of `bytes` needs, which starts it.
    fn version_needed(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[6], bytes[7]])
    }

    /// The version made by field of that same header, the host of the member and its version.
    fn version_made_by(bytes: &[u8]) -> u16 {
        u16::from_le_bytes([bytes[4], bytes[5]])
    }

    /// An archive of the central directory header of `record` and nothing else.
    ///
    /// The sizes and the offsets of a zip64 archive cannot be written in a test, which would
    /// have to hold more than 4 GB of members, so a header is written for sizes and an offset
    /// which are only declared, and the central directory reader is handed it alone. That is
    /// what a reader of such an archive does with the header too: the sizes and the offset of a
    /// member are the ones of the central directory, and nothing else is read for them.
    fn central_only(record: &Record) -> Vec<u8> {
        let mut writer = Writer::new();
        record.write_central(&mut writer.central);
        writer.count = 1;
        writer.finish()
    }

    /// A record of a member of `compressed` bytes which holds `uncompressed`, at `offset`.
    ///
    /// The metadata is the one of a new member, which the tests of the sizes and the offsets
    /// do not look at.
    fn record(name: &str, compressed: u64, uncompressed: u64, offset: u64) -> Record<'_> {
        Record {
            name,
            name_len: name_length(name).unwrap(),
            method: DEFLATE,
            crc32: 0x1234_5678,
            compressed_size: compressed,
            uncompressed_size: uncompressed,
            offset,
            metadata: Metadata::NEW,
        }
    }

    /// An archive of no members is the end-of-central-directory record and nothing else.
    #[test]
    fn writes_an_empty_archive() {
        let bytes = Writer::new().finish();
        assert_eq!(bytes.len(), 22);
        assert!(members(&bytes).is_empty());
    }

    /// A copied member keeps its time and its mode, one the writer compresses gets the epoch.
    #[test]
    fn carries_the_metadata_of_a_copied_member() {
        // 12:34:56 on 2026-01-01, the time `scripts/zip_edge_fixtures.py` writes, and an
        // executable file, which is not the mode the writer gives a member of its own
        let metadata = Metadata {
            time: 0x645c,
            date: 0x5c21,
            attributes: 0o100_755 << 16,
        };
        let member = Member {
            name: "copied.bin".to_string(),
            method: STORED,
            crc32: crc32fast::hash(b"x"),
            compressed_size: 1,
            uncompressed_size: 1,
            header_offset: 0,
            flags: 0,
            dos_time: metadata.time,
            dos_date: metadata.date,
            external_attributes: metadata.attributes,
        };
        let mut writer = Writer::new();
        writer.add_raw("copied.bin", &member, b"x").unwrap();
        writer.add_stored("stored.bin", b"x").unwrap();
        writer.add_deflated("deflated.bin", b"x", 6).unwrap();
        let bytes = writer.finish();
        // the local header carries the time at offset 10 and the date at offset 12
        assert_eq!(bytes[10..14], [0x5c, 0x64, 0x21, 0x5c]);
        let written = members(&bytes);
        assert_eq!(
            Metadata::of(&written[0]),
            metadata,
            "the copy lost its metadata"
        );
        assert_eq!(Metadata::of(&written[1]), Metadata::NEW);
        assert_eq!(Metadata::of(&written[2]), Metadata::NEW);
    }

    /// The version made by field is raised to 4.5 with the version a reader needs.
    ///
    /// Both fields of the central header say 4.5 for a member of the zip64 extra field, which
    /// is how `zipfile` writes them: it raises the create version and the extract version of a
    /// member together.
    #[test]
    fn raises_the_version_made_by_of_a_zip64_member() {
        let small = central_only(&record("small.bin", 1, 1, 0));
        assert_eq!(version_made_by(&small), UNIX | VERSION);
        assert_eq!(version_needed(&small), VERSION);
        for large in [
            central_only(&record("sizes.bin", SENTINEL32_U64, SENTINEL32_U64, 0)),
            central_only(&record("offset.bin", 1, 1, SENTINEL32_U64)),
        ] {
            assert_eq!(version_made_by(&large), UNIX | ZIP64_VERSION);
            assert_eq!(version_needed(&large), ZIP64_VERSION);
        }
    }

    /// The host of a member is unix where its attributes hold a mode, and MS-DOS where not.
    #[test]
    fn names_the_host_which_belongs_to_the_attributes_of_a_member() {
        let mut member = record("member.bin", 1, 1, 0);
        assert_eq!(version_made_by(&central_only(&member)), UNIX | VERSION);
        // what a member of an archive written on Windows or by a java library carries: no mode
        // at all, or only the MS-DOS attribute byte of a file which was archived
        for attributes in [0, 0x20] {
            member.metadata.attributes = attributes;
            assert_eq!(version_made_by(&central_only(&member)), MS_DOS | VERSION);
            assert_eq!(
                members(&central_only(&member))[0].external_attributes,
                attributes
            );
        }
    }

    /// A size or an offset no 32 bit field holds is written into the zip64 extra field.
    #[test]
    fn writes_the_zip64_extra_field_of_a_large_member() {
        for (uncompressed, compressed, offset) in [
            (SENTINEL32_U64, 1, 0),
            (1, SENTINEL32_U64, 0),
            (SENTINEL32_U64 + 7, SENTINEL32_U64 + 3, 0),
            (1, 1, SENTINEL32_U64),
            (1, 1, SENTINEL32_U64 + 9),
            (SENTINEL32_U64 + 1, 4, SENTINEL32_U64 + 9),
        ] {
            let what = format!("{uncompressed}/{compressed} at {offset}");
            let bytes = central_only(&record("big.bin", compressed, uncompressed, offset));
            let member = members(&bytes).pop().unwrap_or_else(|| panic!("{what}"));
            assert_eq!(member.name, "big.bin", "{what}");
            assert_eq!(member.method, DEFLATE, "{what}");
            assert_eq!(member.crc32, 0x1234_5678, "{what}");
            assert_eq!(member.uncompressed_size, uncompressed, "{what}");
            assert_eq!(member.compressed_size, compressed, "{what}");
            assert_eq!(member.header_offset, offset, "{what}");
            assert_eq!(version_needed(&bytes), ZIP64_VERSION, "{what}");
        }
    }

    /// A member of a size which fits carries no zip64 extra field at all.
    #[test]
    fn writes_no_zip64_extra_field_for_a_member_which_needs_none() {
        let bytes = central_only(&record(
            "small.bin",
            SENTINEL32_U64 - 1,
            SENTINEL32_U64 - 1,
            7,
        ));
        let member = members(&bytes).pop().unwrap();
        assert_eq!(member.compressed_size, SENTINEL32_U64 - 1);
        assert_eq!(member.uncompressed_size, SENTINEL32_U64 - 1);
        assert_eq!(member.header_offset, 7);
        // the central header, its name and the record which closes the archive, no extra field
        assert_eq!(bytes.len(), 46 + "small.bin".len() + 22);
        assert_eq!(version_needed(&bytes), VERSION);
    }

    /// The local file header of a large member carries the sentinels and both sizes.
    #[test]
    fn writes_the_zip64_extra_field_of_a_local_header() {
        let record = record("big.bin", SENTINEL32_U64 + 3, SENTINEL32_U64 + 7, 0);
        let mut bytes = Vec::new();
        record.write_local(&mut bytes);
        assert_eq!(bytes[18..26], [0xff; 8], "the sizes are not the sentinel");
        let extra = &bytes[30 + "big.bin".len()..];
        assert_eq!(
            extra[..4],
            [1, 0, 16, 0],
            "no zip64 extra field of 16 bytes"
        );
        assert_eq!(
            u64::from_le_bytes(extra[4..12].try_into().unwrap()),
            SENTINEL32_U64 + 7
        );
        assert_eq!(
            u64::from_le_bytes(extra[12..20].try_into().unwrap()),
            SENTINEL32_U64 + 3
        );
        // the data of the member starts behind the name and the extra field
        let member = members(&central_only(&record)).pop().unwrap();
        assert_eq!(
            local::data_offset(&bytes, &member).unwrap(),
            30 + "big.bin".len() + 20
        );
    }

    /// A name which is not ASCII is written as UTF-8 under bit 11, and read back as it was.
    #[test]
    fn writes_a_name_which_is_not_ascii() {
        let mut writer = Writer::new();
        writer.add_stored("Hühnerstall/±.xml", b"x").unwrap();
        let member = members(&writer.finish()).pop().unwrap();
        assert_eq!(member.name, "Hühnerstall/±.xml");
        assert_eq!(member.flags, UTF8_NAME);
    }

    /// Deflate and inflate are each other's inverse, over every level deflate knows.
    #[test]
    fn deflates_what_inflates_again() {
        let data = b"a member of a COMBINE archive, deflated and inflated again. ".repeat(400);
        for level in 0..=MAX_LEVEL {
            let mut writer = Writer::new();
            writer.add_deflated("m.bin", &data, level).unwrap();
            let reader = Reader::new(writer.finish()).unwrap();
            let member = reader.members().first().unwrap();
            assert_eq!(member.method, DEFLATE, "level {level}");
            assert_eq!(reader.inflate(member).unwrap(), data, "level {level}");
        }
        // an empty member deflates to the one empty stored block of a deflate stream
        assert_eq!(deflate(b"", 6).unwrap(), [0x03, 0x00]);
    }

    /// A level deflate does not know is refused before it reaches the compressor.
    #[test]
    fn refuses_a_level_deflate_does_not_know() {
        for level in [MAX_LEVEL + 1, u32::MAX] {
            assert!(matches!(deflate(b"x", level), Err(OmexError::Refused(_))));
        }
    }
}
