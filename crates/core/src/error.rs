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
}
