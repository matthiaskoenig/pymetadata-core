//! Lazy reading and writing of COMBINE archives (OMEX).

pub mod error;
pub mod formats;
mod formats_table;
pub mod location;

pub use error::OmexError;
pub use formats::{UNKNOWN_FORMAT, guess_format, lookup_format};
pub use location::normalize_location;
