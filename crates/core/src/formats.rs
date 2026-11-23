//! `lookup_format` and `guess_format` over the generated format tables.

use crate::formats_table::{FORMATS, XML_ROOT_FORMATS};

/// The URI of an unknown media type, the fallback of [`lookup_format`] and [`guess_format`].
pub const UNKNOWN_FORMAT: &str = "https://purl.org/NET/mediatypes/application/x.unknown";

/// How many bytes of a file `guess_format` looks at for the root tag, as in python.
const HEAD_LEN: usize = 256;

/// Look up the format URI for a format key, e.g. `sbml` or `csv`.
///
/// The key is the name of an `EntryFormat` member of `pymetadata.omex` and is
/// matched without regard to case. An unknown key gives [`UNKNOWN_FORMAT`].
pub fn lookup_format(format_key: &str) -> &'static str {
    FORMATS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(format_key))
        .map_or(UNKNOWN_FORMAT, |(_, uri)| uri)
}

/// Guess the format URI of a file from its suffix and its first bytes.
///
/// `suffix` is the file suffix without the dot. For `xml` files the start tags
/// in the first 256 bytes of `head` tell SBML, SED-ML, CellML and COPASI apart;
/// for every other file, and for an `xml` file without such a tag, the suffix
/// decides. The host reads the file, the core never touches the filesystem.
pub fn guess_format(suffix: &str, head: &[u8]) -> &'static str {
    if suffix.eq_ignore_ascii_case("xml") {
        let head = head.get(..HEAD_LEN).unwrap_or(head);
        for (tag, key) in XML_ROOT_FORMATS {
            if head.windows(tag.len()).any(|window| window == *tag) {
                return lookup_format(key);
            }
        }
    }
    lookup_format(suffix)
}
