//! The central directory, the one record which describes every member of an archive.
//!
//! The central directory is read in full when an archive is opened, and it is the authority on
//! a member: the local file header in front of the data repeats its fields and is allowed to
//! leave them zero when the member was written through a stream.

use super::eocd::Eocd;
use super::{OmexError, as_usize, end_of, has_signature, slice_at, u16_at, u32_at, u64_at};

/// The signature of a central directory file header.
const SIGNATURE: [u8; 4] = *b"PK\x01\x02";
/// The size of a central directory file header, its name, extra field and comment excluded.
const FIXED_SIZE: usize = 46;
/// The header id of the zip64 extended information extra field.
const ZIP64_EXTRA: u16 = 0x0001;
/// The value a 32 bit size or offset carries when the zip64 extra field holds the real one.
const SENTINEL32: u32 = 0xffff_ffff;
/// Bit 11 of the general purpose flags: the member name is UTF-8 and not CP437.
const UTF8_NAME: u16 = 0x0800;

/// The upper half of CP437, the code page a member name without bit 11 is written in.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', //
    'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', //
    'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', //
    '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', //
    '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', //
    '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', //
    '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', //
    'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', //
    '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// One member of a zip archive, as the central directory describes it.
///
/// The sizes and the CRC32 are the ones of the central directory, which hold where the local
/// file header carries zeros, and they are what the writer copies with a raw member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The name of the member, a relative path with `/` as its separator.
    pub name: String,
    /// The compression method: 0 stored, 8 deflate, and nothing else is supported.
    pub method: u16,
    /// The CRC32 of the uncompressed bytes.
    pub crc32: u32,
    /// The number of bytes the member occupies in the archive.
    pub compressed_size: u64,
    /// The number of bytes the member holds uncompressed.
    pub uncompressed_size: u64,
    /// The offset of the local file header of the member from the start of the archive.
    pub header_offset: u64,
    /// The general purpose bit flags: bit 0 encryption, bit 3 a data descriptor, bit 11 UTF-8.
    pub flags: u16,
    /// The modification time in MS-DOS format, whose high byte checks the password of an
    /// encrypted member written through a stream.
    pub dos_time: u16,
}

/// Read every member the central directory of `bytes` lists.
///
/// The local header offset of a member is an offset from the start of the archive, so the
/// bytes in front of the archive are added to it and every offset a [`Member`] carries is one
/// into the bytes the reader was handed.
///
/// # Errors
///
/// [`OmexError::Malformed`] if a file header is missing, reaches past the end of the archive or
/// lacks a zip64 extra field a field of all ones needs.
pub(super) fn members(bytes: &[u8], eocd: &Eocd) -> Result<Vec<Member>, OmexError> {
    let mut cursor = as_usize(eocd.cd_offset, "the offset of the central directory")?;
    let mut members = Vec::new();
    // the declared count is host controlled and can be huge, but every member read costs at
    // least `FIXED_SIZE` bytes of the archive, so the count cannot drive work or allocation
    // beyond what the input holds: the signature check below ends the loop
    for index in 0..eocd.entries {
        if !has_signature(bytes, cursor, &SIGNATURE) {
            return Err(OmexError::Malformed(format!(
                "the central directory has no file header for member {index} at offset {cursor}"
            )));
        }
        let (mut member, next) = read(bytes, cursor)?;
        member.header_offset = member
            .header_offset
            .checked_add(eocd.prepended)
            .ok_or_else(|| {
                OmexError::Malformed(format!(
                    "the local header of member {:?} is at offset {} behind the {} bytes in \
                     front of the archive, which does not fit",
                    member.name, member.header_offset, eocd.prepended
                ))
            })?;
        members.push(member);
        cursor = next;
    }
    Ok(members)
}

/// Read the file header at `offset` and say where the next one starts.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the header reaches past the end of the archive, if its name is
/// no valid UTF-8 although bit 11 says so, or if a zip64 extra field it needs is missing.
fn read(bytes: &[u8], offset: usize) -> Result<(Member, usize), OmexError> {
    let flags = u16_at(bytes, end_of(offset, 8)?)?;
    let method = u16_at(bytes, end_of(offset, 10)?)?;
    let dos_time = u16_at(bytes, end_of(offset, 12)?)?;
    let crc32 = u32_at(bytes, end_of(offset, 16)?)?;
    let compressed_size = u32_at(bytes, end_of(offset, 20)?)?;
    let uncompressed_size = u32_at(bytes, end_of(offset, 24)?)?;
    let name_len = usize::from(u16_at(bytes, end_of(offset, 28)?)?);
    let extra_len = usize::from(u16_at(bytes, end_of(offset, 30)?)?);
    let comment_len = usize::from(u16_at(bytes, end_of(offset, 32)?)?);
    let header_offset = u32_at(bytes, end_of(offset, 42)?)?;
    let name_at = end_of(offset, FIXED_SIZE)?;
    let extra_at = end_of(name_at, name_len)?;
    let comment_at = end_of(extra_at, extra_len)?;
    let mut member = Member {
        name: decode_name(slice_at(bytes, name_at, name_len)?, flags)?,
        method,
        crc32,
        compressed_size: compressed_size.into(),
        uncompressed_size: uncompressed_size.into(),
        header_offset: header_offset.into(),
        flags,
        dos_time,
    };
    let sentinel = [compressed_size, uncompressed_size, header_offset].contains(&SENTINEL32);
    let extra = slice_at(bytes, extra_at, extra_len)?;
    if sentinel {
        zip64(extra, &mut member)?;
    }
    // the comment is not kept, but it has to be inside the archive for the next header to follow
    let comment = slice_at(bytes, comment_at, comment_len)?;
    Ok((member, end_of(comment_at, comment.len())?))
}

/// Decode a member name, as python's `zipfile` does: UTF-8 under bit 11, CP437 without it.
///
/// # Errors
///
/// [`OmexError::Malformed`] if bit 11 claims UTF-8 and the bytes are not.
fn decode_name(raw: &[u8], flags: u16) -> Result<String, OmexError> {
    if flags & UTF8_NAME == 0 {
        return Ok(raw
            .iter()
            .map(|byte| {
                if byte.is_ascii() {
                    return char::from(*byte);
                }
                // 0x80..=0xff lands in the 128 entries of the table, and `get` keeps that
                // checked rather than leaving it to the reader of this line
                CP437_HIGH
                    .get(usize::from(*byte) - 0x80)
                    .copied()
                    .unwrap_or(char::REPLACEMENT_CHARACTER)
            })
            .collect());
    }
    String::from_utf8(raw.to_vec()).map_err(|error| {
        OmexError::Malformed(format!(
            "a member name is no valid UTF-8 although bit 11 of its flags says so: {error}"
        ))
    })
}

/// Take the sizes and the local header offset which the zip64 extra field of `extra` carries.
///
/// The field holds only the values whose 32 bit counterpart is all ones, in the order of the
/// file header, which is why it is read against the member it belongs to.
///
/// # Errors
///
/// [`OmexError::Malformed`] if an extra field reaches past the end of the extra fields, or if
/// the zip64 field is too short for the values the file header needs from it.
fn zip64(extra: &[u8], member: &mut Member) -> Result<(), OmexError> {
    let mut cursor = 0;
    while end_of(cursor, 4)? <= extra.len() {
        let id = u16_at(extra, cursor)?;
        let len = usize::from(u16_at(extra, end_of(cursor, 2)?)?);
        let data = slice_at(extra, end_of(cursor, 4)?, len)?;
        if id != ZIP64_EXTRA {
            cursor = end_of(end_of(cursor, 4)?, len)?;
            continue;
        }
        let mut at = 0;
        if member.uncompressed_size == u64::from(SENTINEL32) {
            member.uncompressed_size = u64_at(data, at)?;
            at = end_of(at, 8)?;
        }
        if member.compressed_size == u64::from(SENTINEL32) {
            member.compressed_size = u64_at(data, at)?;
            at = end_of(at, 8)?;
        }
        if member.header_offset == u64::from(SENTINEL32) {
            member.header_offset = u64_at(data, at)?;
        }
        return Ok(());
    }
    Err(OmexError::Malformed(format!(
        "member {:?} needs a zip64 extended information extra field, which it has not",
        member.name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A member name is UTF-8 under bit 11 of the flags and CP437 without it.
    #[test]
    fn decodes_names_like_zipfile() {
        let name = "Hühnerstall/±.xml";
        assert_eq!(decode_name(name.as_bytes(), UTF8_NAME).unwrap(), name);
        assert_eq!(decode_name(b"plain.xml", 0).unwrap(), "plain.xml");
        // the bytes python's `zipfile` writes for the same name without bit 11
        assert_eq!(decode_name(b"H\x81hnerstall/\xf1.xml", 0).unwrap(), name);
        // CP437 cannot fail, UTF-8 can
        assert!(decode_name(b"\x81\xf1", 0).is_ok());
        assert!(matches!(
            decode_name(b"\x81\xf1", UTF8_NAME),
            Err(OmexError::Malformed(_))
        ));
    }
}
