//! The local file header, which says where the data of a member starts.

use super::spec::{LOCAL_FIXED_SIZE, LOCAL_SIGNATURE};
use super::{Member, OmexError, as_usize, end_of, has_signature, u16_at};

/// Find where the data of `member` starts.
///
/// Only the two lengths of the header are read. Its sizes and its CRC32 are zero whenever the
/// member was written through a stream, which sets bit 3 of the flags and puts a data descriptor
/// behind the data, and its extra field can differ from the one of the central directory, so
/// every size comes from the central directory and none from here.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the archive holds no local file header at the offset the central
/// directory gives, or if the header reaches past the end of the archive.
pub(super) fn data_offset(bytes: &[u8], member: &Member) -> Result<usize, OmexError> {
    let header = as_usize(member.header_offset, "the offset of the local file header")?;
    if !has_signature(bytes, header, &LOCAL_SIGNATURE) {
        return Err(OmexError::Malformed(format!(
            "member {:?} has no local file header at offset {header}",
            member.name
        )));
    }
    let name_len = usize::from(u16_at(bytes, end_of(header, 26)?)?);
    let extra_len = usize::from(u16_at(bytes, end_of(header, 28)?)?);
    let fixed = end_of(header, LOCAL_FIXED_SIZE)?;
    end_of(end_of(fixed, name_len)?, extra_len)
}
