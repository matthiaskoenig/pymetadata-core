//! The zip container of a COMBINE archive, read and written by hand.
//!
//! The `zip` crate cannot be used: it declares `getrandom` with the `wasm_js` feature as a hard
//! dependency for `wasm32`, which pulls in `wasm-bindgen` and gives the wasm module six imports
//! where it must have none (`spike/RESULTS.md`). The records are simple enough to read, and the
//! writer needs the raw members anyway, so the core owns the container.
//!
//! Everything here works on bytes in memory and never touches the filesystem. Every length,
//! offset and count is read from host controlled bytes, so it is checked against the buffer
//! before it is used: a crafted archive is an [`OmexError`] and never a panic, which would trap
//! a wasm instance.
//!
//! One module per record, and `spec` for the constants the format prescribes, which the reader
//! and the writer both speak and therefore declare once.

mod central;
mod crypto;
mod eocd;
mod local;
pub mod probe;
mod reader;
mod spec;
mod writer;

pub use central::Member;
pub use reader::{MAX_INFLATED_SIZE, Reader};
pub use writer::Writer;

use crate::OmexError;

/// Read `N` bytes of `bytes` at `offset`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the bytes are not there.
fn array_at<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], OmexError> {
    // `slice_at` hands back exactly `N` bytes, so the conversion holds; it is written as a
    // fallible one all the same, so that no `unwrap` on host controlled bytes is left anywhere
    slice_at(bytes, offset, N)?
        .try_into()
        .map_err(|_| truncated(offset, N, bytes.len()))
}

/// Read the little endian `u16` of `bytes` at `offset`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the bytes are not there.
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, OmexError> {
    Ok(u16::from_le_bytes(array_at(bytes, offset)?))
}

/// Read the little endian `u32` of `bytes` at `offset`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the bytes are not there.
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, OmexError> {
    Ok(u32::from_le_bytes(array_at(bytes, offset)?))
}

/// Read the little endian `u64` of `bytes` at `offset`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the bytes are not there.
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, OmexError> {
    Ok(u64::from_le_bytes(array_at(bytes, offset)?))
}

/// Borrow `len` bytes of `bytes` at `offset`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the bytes are not there.
fn slice_at(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8], OmexError> {
    let end = end_of(offset, len)?;
    bytes
        .get(offset..end)
        .ok_or_else(|| truncated(offset, len, bytes.len()))
}

/// The end of a record of `len` bytes at `offset`.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the sum does not fit the address space.
fn end_of(offset: usize, len: usize) -> Result<usize, OmexError> {
    offset.checked_add(len).ok_or_else(|| {
        OmexError::Malformed(format!(
            "a record of {len} bytes at offset {offset} is past the address space"
        ))
    })
}

/// Does `bytes` carry `signature` at `offset`?
fn has_signature(bytes: &[u8], offset: usize, signature: &[u8; 4]) -> bool {
    slice_at(bytes, offset, signature.len()).is_ok_and(|found| found == signature)
}

/// Narrow an offset or a length of the container to a `usize` of this platform.
///
/// wasm32 is a 32 bit target, so a zip64 archive can hold an offset the host cannot address.
///
/// # Errors
///
/// [`OmexError::Malformed`] if the value does not fit a `usize`.
fn as_usize(value: u64, what: &str) -> Result<usize, OmexError> {
    usize::try_from(value)
        .map_err(|_| OmexError::Malformed(format!("{what} {value} does not fit the address space")))
}

/// The error of a record which reaches past the end of the archive.
fn truncated(offset: usize, len: usize, total: usize) -> OmexError {
    OmexError::Malformed(format!(
        "the archive of {total} bytes is truncated: it has no {len} bytes at offset {offset}"
    ))
}
