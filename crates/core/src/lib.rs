//! Lazy reading and writing of COMBINE archives (OMEX).

pub mod archive;
pub mod error;
pub mod formats;
mod formats_table;
pub mod location;
pub mod manifest;
pub mod wire;
pub mod zip;

pub use archive::{Archive, MemberInfo};
pub use error::OmexError;
pub use formats::{UNKNOWN_FORMAT, guess_format, lookup_format};
pub use location::normalize_location;
pub use manifest::{Entry, Manifest};
