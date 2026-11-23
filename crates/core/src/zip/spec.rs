//! The constants of the zip format itself, read and written by the modules beside this one.
//!
//! Every value here is one the format prescribes: the signatures of the records, how long their
//! fixed parts are, the compression methods, the bits of the general purpose flags, the id of
//! the zip64 extra field, the sentinels which say that a 32 bit field is not the real value, and
//! the two versions a reader needs. They live in one module because the reader and the writer
//! both speak them, and a drift between the reader's notion of a flag bit and the writer's would
//! be a silent interop bug in the one module where a bug corrupts a user's archive. Declared
//! once, there is nothing to drift.
//!
//! What is not here is what only one module speaks, whether the format prescribes it or not: the
//! inflate budget of the reader ([`super::MAX_INFLATED_SIZE`]), the deflate ceiling and the buffer steps,
//! and the modification time and the mode the writer stamps on a member it compresses itself.

/// The signature of a local file header.
pub(super) const LOCAL_SIGNATURE: [u8; 4] = *b"PK\x03\x04";
/// The signature of a central directory file header.
pub(super) const CENTRAL_SIGNATURE: [u8; 4] = *b"PK\x01\x02";
/// The signature of an end-of-central-directory record.
pub(super) const EOCD_SIGNATURE: [u8; 4] = *b"PK\x05\x06";
/// The signature of a zip64 end-of-central-directory record.
pub(super) const ZIP64_EOCD_SIGNATURE: [u8; 4] = *b"PK\x06\x06";
/// The signature of the locator of a zip64 end-of-central-directory record.
pub(super) const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = *b"PK\x06\x07";

/// The size of a local file header, its name and extra field excluded.
pub(super) const LOCAL_FIXED_SIZE: usize = 30;
/// The size of a central directory file header, its name, extra field and comment excluded.
pub(super) const CENTRAL_FIXED_SIZE: usize = 46;
/// The size of an end-of-central-directory record, its comment excluded.
pub(super) const EOCD_SIZE: usize = 22;
/// The size of a zip64 end-of-central-directory record without an extensible data sector.
pub(super) const ZIP64_EOCD_SIZE: usize = 56;
/// The size of the locator of that record, which sits right in front of the record it belongs to.
pub(super) const ZIP64_LOCATOR_SIZE: usize = 20;
/// The largest archive comment, which bounds the backward scan for the record.
pub(super) const MAX_COMMENT: usize = 0xffff;

/// The stored method: the bytes of the member are its content.
pub(super) const STORED: u16 = 0;
/// The deflate method, which is what every other member of a COMBINE archive uses.
pub(super) const DEFLATE: u16 = 8;

/// Bit 0 of the general purpose flags: the member is encrypted.
pub(super) const ENCRYPTED: u16 = 0x0001;
/// Bit 3 of the general purpose flags: sizes and CRC32 follow the data in a data descriptor.
pub(super) const DATA_DESCRIPTOR: u16 = 0x0008;
/// Bit 6 of the general purpose flags: strong encryption, which is not ZipCrypto.
pub(super) const STRONG_ENCRYPTION: u16 = 0x0040;
/// Bit 11 of the general purpose flags: the member name is UTF-8 and not CP437.
pub(super) const UTF8_NAME: u16 = 0x0800;

/// The header id of the zip64 extended information extra field.
pub(super) const ZIP64_EXTRA: u16 = 0x0001;

/// The value a 16 bit count carries when the zip64 record holds the real one.
///
/// A count of exactly this value needs the zip64 record all the same: the value is the sentinel
/// which says that the real one is in that record, so a reader which met it in the 16 bit field
/// would look for a record which is not there. The other way round, 0xffff members and a 4 GB
/// offset are legal values and not only sentinels, which is why the 32 bit record stands where
/// no zip64 record backs it.
pub(super) const SENTINEL16: u16 = u16::MAX;
/// The value a 32 bit size or offset carries when a zip64 record holds the real one.
///
/// As for [`SENTINEL16`], the value itself is the sentinel.
pub(super) const SENTINEL32: u32 = u32::MAX;
/// [`SENTINEL16`] in the width the writer keeps its member count in.
pub(super) const SENTINEL16_U64: u64 = SENTINEL16 as u64;
/// [`SENTINEL32`] in the width the writer keeps its sizes and its offsets in.
pub(super) const SENTINEL32_U64: u64 = SENTINEL32 as u64;

/// The version a reader needs for an ordinary member, 2.0, which is deflate.
pub(super) const VERSION: u16 = 20;
/// The version a reader needs for the zip64 records, 4.5.
pub(super) const ZIP64_VERSION: u16 = 45;
