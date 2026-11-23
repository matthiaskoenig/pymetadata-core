//! The lazy reader of a zip container.

use std::borrow::Cow;

use flate2::{Decompress, FlushDecompress, Status};

use super::crypto;
use super::eocd::Eocd;
use super::spec::{DEFLATE, ENCRYPTED, STORED, STRONG_ENCRYPTION};
use super::{Member, OmexError, as_usize, central, local, slice_at};

/// The largest factor by which deflate can expand data, which bounds a declared size.
const MAX_EXPANSION: u64 = 1032;
/// The step the buffer of a member grows by while it inflates, before it doubles.
const INFLATE_CHUNK: usize = 64 * 1024;

/// The number of uncompressed bytes [`Reader::new`] inflates for one member at most, 1 GiB.
///
/// A member declares how large it is uncompressed, and that number is host controlled: deflate
/// expands by up to 1032:1, so a crafted archive of a few MB can declare, and genuinely
/// inflate to, several GB. wasm32 has 4 GB of address space in all, an allocation which fails
/// aborts, and an abort traps the instance, so the ceiling has to sit well below it. 1 GiB is
/// about 150 times the largest member of the COMBINE archives of the test corpus (7.1 MB
/// inflated) and far below what wasm32 can address. A host with another budget passes it to
/// [`Reader::with_max_inflated_size`].
pub const MAX_INFLATED_SIZE: usize = 1 << 30;

/// A zip archive over the bytes it owns, which inflates a member only when it is asked for.
///
/// [`Reader::new`] reads the central directory and nothing else, so listing the contents of an
/// archive or reading its manifest alone costs no inflation at all. The reader holds no
/// inflated bytes; caching them belongs to whoever asks for them.
#[derive(Clone)]
pub struct Reader {
    /// The whole archive.
    bytes: Vec<u8>,
    /// Its members, in the order of the central directory.
    members: Vec<Member>,
    /// The number of uncompressed bytes one member may inflate to.
    max_inflated_size: usize,
    /// The password of the encrypted members, if the host has one.
    password: Option<Vec<u8>>,
}

impl Reader {
    /// Read the central directory of the archive in `bytes`.
    ///
    /// A member of more than [`MAX_INFLATED_SIZE`] uncompressed bytes is refused;
    /// [`Reader::with_max_inflated_size`] is the same reader under another budget.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the bytes are no zip archive, or if a record of the container
    /// reaches past their end.
    pub fn new(bytes: Vec<u8>) -> Result<Self, OmexError> {
        Self::open(bytes)
    }

    /// Read the central directory of the archive in `bytes`, inflating `limit` bytes at most.
    ///
    /// The limit is per member and belongs to the host, because what a host can afford to
    /// inflate is the host's to say: a machine has its memory, and a wasm instance has 4 GB of
    /// address space in which an allocation that fails traps it. Both hosts of this workspace
    /// take the default of [`MAX_INFLATED_SIZE`] and set none of their own, which
    /// `crates/wasm/src/calls.rs` says of the wasm one, so this is the knob of a host which
    /// needs a tighter ceiling than 1 GiB rather than a budget either of them chose.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the bytes are no zip archive, or if a record of the container
    /// reaches past their end.
    pub fn with_max_inflated_size(bytes: Vec<u8>, limit: usize) -> Result<Self, OmexError> {
        let mut reader = Self::open(bytes)?;
        reader.max_inflated_size = limit;
        Ok(reader)
    }

    /// Read the central directory of the archive in `bytes`, under the default allowance and
    /// without a password.
    fn open(bytes: Vec<u8>) -> Result<Self, OmexError> {
        let eocd = Eocd::find(&bytes)?;
        let members = central::members(&bytes, &eocd)?;
        Ok(Self {
            bytes,
            members,
            max_inflated_size: MAX_INFLATED_SIZE,
            password: None,
        })
    }

    /// Read the central directory of the archive in `bytes`, decrypting with `password`.
    ///
    /// The password is bytes and not text, as it is for python's `zipfile`. A member which is not
    /// encrypted is read as it is, and the password is ignored for it. An empty password is no
    /// password, as it is for `zipfile`, where `setpassword` maps a falsy one to none.
    /// [`Reader::set_max_inflated_size`] combines it with another allowance.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the bytes are no zip archive, or if a record of the container
    /// reaches past their end.
    pub fn new_with_password(bytes: Vec<u8>, password: &[u8]) -> Result<Self, OmexError> {
        let mut reader = Self::new(bytes)?;
        reader.password = (!password.is_empty()).then(|| password.to_vec());
        Ok(reader)
    }

    /// Change how many uncompressed bytes the reader inflates for one member at most.
    ///
    /// This is [`Reader::with_max_inflated_size`] for a reader which is built another way, such
    /// as [`Reader::new_with_password`].
    pub fn set_max_inflated_size(&mut self, limit: usize) {
        self.max_inflated_size = limit;
    }

    /// The members of the archive, in the order of the central directory.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The bytes of `member` as the archive stores them, compressed as they are.
    ///
    /// This is what the writer copies when it takes an untouched member over into a new
    /// archive, which is why the bytes are borrowed from the reader and not copied here.
    ///
    /// # Errors
    ///
    /// [`OmexError::Malformed`] if the member has no local file header, or if the archive is
    /// shorter than the compressed size the central directory declares.
    pub fn raw(&self, member: &Member) -> Result<&[u8], OmexError> {
        let offset = local::data_offset(&self.bytes, member)?;
        let size = as_usize(member.compressed_size, "the compressed size of a member")?;
        slice_at(&self.bytes, offset, size)
    }

    /// The uncompressed bytes of `member`, checked against its size and its CRC32.
    ///
    /// An encrypted member is decrypted first, into a buffer of its own, and `raw` is left as
    /// the archive stores it.
    ///
    /// # Errors
    ///
    /// [`OmexError::Refused`] if the member inflates to more than this reader allows, or is
    /// encrypted and the reader has no password or the wrong one,
    /// [`OmexError::Unsupported`] if the member uses a compression method other than stored or
    /// deflate (WinZip AES is method 99) or strong encryption, [`OmexError::Malformed`] if the
    /// member cannot be read, does not inflate, or does not match the size and the CRC32 of
    /// the central directory.
    pub fn inflate(&self, member: &Member) -> Result<Vec<u8>, OmexError> {
        let raw = self.raw(member)?;
        if !matches!(member.method, STORED | DEFLATE) {
            let method = member.method;
            let known = method_name(method).map_or(String::new(), |name| format!(" ({name})"));
            return Err(OmexError::Unsupported(format!(
                "member {:?} uses compression method {method}{known}",
                member.name
            )));
        }
        if member.flags & STRONG_ENCRYPTION != 0 {
            return Err(OmexError::Unsupported(format!(
                "member {:?} uses strong encryption, which is not ZipCrypto",
                member.name
            )));
        }
        let stream = if member.flags & ENCRYPTED == 0 {
            Cow::Borrowed(raw)
        } else {
            let password = self.password.as_deref().ok_or_else(|| {
                OmexError::Refused(format!(
                    "member {:?} is encrypted and the reader was given no password",
                    member.name
                ))
            })?;
            Cow::Owned(crypto::decrypt(member, raw, password)?)
        };
        let data = match member.method {
            // a stored member is its own uncompressed bytes, which the budget covers too; a
            // declared size which is another one is caught by `verify`
            STORED if stream.len() > self.max_inflated_size => {
                return Err(refuse(member, stream.len() as u64, self.max_inflated_size));
            }
            STORED => stream.into_owned(),
            _ => decompress(&stream, member, self.max_inflated_size)?,
        };
        verify(member, &data)?;
        Ok(data)
    }
}

impl std::fmt::Debug for Reader {
    /// Show how large the archive is and how many members it has, and not its bytes.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Reader")
            .field("bytes", &self.bytes.len())
            .field("members", &self.members.len())
            .field("max_inflated_size", &self.max_inflated_size)
            .finish()
    }
}

/// Inflate the deflate stream of `member`, into at most `limit` bytes.
///
/// The buffer grows in bounded steps and never past the end the member declares, so a stream
/// which inflates to more than it promised stops one byte behind that end and is caught by
/// [`verify`], and the declared size never drives an allocation on its own.
///
/// This drives `flate2::Decompress` and not the reader flate2 offers over the `Read` trait of
/// the standard library: the core uses neither the filesystem nor the stream traits, because a
/// host hands it bytes and takes bytes back.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the stream is corrupt, ends early, or if the declared
/// uncompressed size is one no deflate stream of this length could carry,
/// [`OmexError::Refused`] if the declared size is one the stream could carry and `limit` is
/// below it.
fn decompress(raw: &[u8], member: &Member, limit: usize) -> Result<Vec<u8>, OmexError> {
    // deflate expands by at most 1032:1, so a declared size far above that is a lie, and a
    // lie is malformed before it is anything else: a host which raised its budget would still
    // not read it. It is refused before it reaches an allocation, like the budget is.
    let bound = (raw.len() as u64)
        .saturating_mul(MAX_EXPANSION)
        .saturating_add(1024);
    if member.uncompressed_size > bound {
        return Err(OmexError::Malformed(format!(
            "member {:?} declares {} uncompressed bytes, which {} compressed bytes cannot hold",
            member.name,
            member.uncompressed_size,
            raw.len()
        )));
    }
    if member.uncompressed_size > limit as u64 {
        return Err(refuse(member, member.uncompressed_size, limit));
    }
    // a stream of no bytes holds an empty member, which is what .NET writes for an empty entry
    // and `zipfile` reads; one which claims bytes goes on to fail like any stream which ends
    // early
    if raw.is_empty() && member.uncompressed_size == 0 {
        return Ok(Vec::new());
    }
    let declared = as_usize(
        member.uncompressed_size,
        "the uncompressed size of a member",
    )?;
    let ceiling = declared.min(limit).saturating_add(1);
    let mut data: Vec<u8> = Vec::new();
    let mut state = Decompress::new(false);
    loop {
        if data.len() == data.capacity() {
            // the buffer doubles, as a vector does, but never beyond the ceiling, so what is
            // committed stays inside the budget of the host and no realloc copies every step
            let grow = data
                .capacity()
                .max(INFLATE_CHUNK)
                .min(ceiling.saturating_sub(data.len()));
            if grow == 0 {
                // the ceiling is reached: `verify` reports the member as longer than declared
                break;
            }
            data.reserve_exact(grow);
        }
        let read = state.total_in();
        let written = state.total_out();
        let consumed = as_usize(read, "the length of a deflate stream")?;
        let rest = raw.get(consumed..).ok_or_else(|| {
            OmexError::Malformed(format!(
                "member {:?} inflated past the {} bytes it holds",
                member.name,
                raw.len()
            ))
        })?;
        let status = state
            .decompress_vec(rest, &mut data, FlushDecompress::None)
            .map_err(|error| {
                OmexError::Malformed(format!(
                    "member {:?} does not inflate: {error}",
                    member.name
                ))
            })?;
        if status == Status::StreamEnd {
            break;
        }
        if state.total_in() == read && state.total_out() == written {
            return Err(OmexError::Malformed(format!(
                "the deflate stream of member {:?} ends before the member does",
                member.name
            )));
        }
    }
    Ok(data)
}

/// The error which refuses a member of `size` bytes as larger than the budget `limit`.
fn refuse(member: &Member, size: u64, limit: usize) -> OmexError {
    OmexError::Refused(format!(
        "member {:?} inflates to {size} bytes, and this reader inflates {limit} bytes at most",
        member.name
    ))
}

/// Check `data` against the size and the CRC32 the central directory gives for `member`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if either differs.
fn verify(member: &Member, data: &[u8]) -> Result<(), OmexError> {
    if data.len() as u64 != member.uncompressed_size {
        return Err(OmexError::Malformed(format!(
            "member {:?} holds {} bytes, the central directory says {}",
            member.name,
            data.len(),
            member.uncompressed_size
        )));
    }
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(data);
    let crc32 = hasher.finalize();
    if crc32 != member.crc32 {
        return Err(OmexError::Malformed(format!(
            "member {:?} has CRC32 {crc32:08x}, the central directory says {:08x}",
            member.name, member.crc32
        )));
    }
    Ok(())
}

/// The name of a compression method, for the error which refuses it.
fn method_name(method: u16) -> Option<&'static str> {
    match method {
        1 => Some("shrink"),
        2..=5 => Some("reduce"),
        6 => Some("implode"),
        9 => Some("enhanced deflate"),
        10 => Some("PKWARE implode"),
        12 => Some("bzip2"),
        14 => Some("lzma"),
        93 => Some("zstd"),
        95 => Some("xz"),
        96 => Some("jpeg"),
        97 => Some("wavpack"),
        98 => Some("ppmd"),
        99 => Some("AES"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::zip::spec::EOCD_SIGNATURE;

    use super::*;

    /// An end-of-central-directory record over `entries` members at `cd_offset`.
    ///
    /// The record of an empty archive is a whole zip archive on its own, which is what the
    /// tests here build on; everything bigger is a fixture of `tests/data/zip/`, written by
    /// python's `zipfile` in `scripts/zip_edge_fixtures.py`.
    fn end_record(entries: u16, cd_offset: u32) -> Vec<u8> {
        let mut bytes = EOCD_SIGNATURE.to_vec();
        bytes.extend_from_slice(&[0; 4]); // this disk, and the disk the directory starts on
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // the size of the central directory
        bytes.extend_from_slice(&cd_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // no archive comment
        bytes
    }

    /// An archive of no members is the end-of-central-directory record and nothing else.
    #[test]
    fn reads_an_empty_archive() {
        let bytes = end_record(0, 0);
        let reader = Reader::new(bytes).unwrap();
        assert!(reader.members().is_empty());
    }

    /// Bytes which hold no end-of-central-directory record are malformed.
    #[test]
    fn refuses_bytes_without_a_record() {
        for bytes in [vec![], vec![0; 21], vec![0x55; 1000]] {
            assert!(
                matches!(Reader::new(bytes.clone()), Err(OmexError::Malformed(_))),
                "{} bytes",
                bytes.len()
            );
        }
    }

    /// A record whose comment length does not match the bytes behind it is read all the same,
    /// which python's `zipfile` does not: its `_EndRecData` requires the length to match and
    /// raises `BadZipFile` for this archive, where the core is deliberately more permissive.
    #[test]
    fn reads_a_record_with_an_inconsistent_comment_length() {
        let mut bytes = end_record(0, 0);
        bytes[20] = 5;
        let reader = Reader::new(bytes).unwrap();
        assert!(reader.members().is_empty());
    }

    /// A central directory which is not where the record says is malformed.
    #[test]
    fn refuses_a_central_directory_which_is_not_there() {
        for offset in [0, 1, 1000, u32::MAX] {
            let bytes = end_record(1, offset);
            assert!(
                matches!(Reader::new(bytes), Err(OmexError::Malformed(_))),
                "offset {offset}"
            );
        }
    }

    /// Every compression method the core refuses is refused by a name a user can look up.
    #[test]
    fn names_the_methods_it_does_not_support() {
        assert_eq!(method_name(12), Some("bzip2"));
        assert_eq!(method_name(99), Some("AES"));
        assert_eq!(method_name(42), None);
        // the two methods it does support are never named
        assert_eq!(method_name(STORED), None);
        assert_eq!(method_name(DEFLATE), None);
    }
}
