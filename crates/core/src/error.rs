//! The error type of the core.

/// Why an operation on an archive failed.
///
/// Malformed input is always one of these and never a panic, since a panic
/// would trap a wasm instance.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OmexError {
    /// The bytes are not a valid archive, zip container or manifest.
    #[error("malformed: {0}")]
    Malformed(String),
    /// The input is refused on purpose, e.g. a location pointing outside of the archive.
    #[error("refused: {0}")]
    Refused(String),
    /// The input is valid but uses a feature the core does not support.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A requested entry does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A member whose password is wrong or missing, which is a refusal of its own kind.
    ///
    /// The message carries the `refused` prefix, since this is a refusal and the prefix is
    /// what hosts have been reading: what changes is that the kind is readable without the
    /// message, see [`crate::wire::kind`]. A host which has to tell a member it may not read
    /// from one which is not there, as `pymetadata` does to raise its own `PasswordError`,
    /// matches on this variant instead of on the text.
    #[error("refused: {0}")]
    Password(String),

    /// A sink which could not take the bytes of a write, e.g. a full disk.
    ///
    /// Nothing about the archive is wrong when this is raised: the bytes were produced and the
    /// sink refused them, so a caller with another sink can write the same archive again. A
    /// write into memory cannot fail this way, so the WebAssembly host never meets it.
    #[error("io: {0}")]
    Io(String),
}
