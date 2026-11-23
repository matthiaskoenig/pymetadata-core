//! Normalization of the location of an archive entry.

use crate::error::OmexError;

/// Normalize the location of an entry in a COMBINE archive.
///
/// Locations are relative POSIX paths starting with `./`, e.g.
/// `./models/model.xml`; the archive itself has the location `.`. A missing
/// `./` prefix is added, backslashes are converted to slashes and empty or `.`
/// segments are removed.
///
/// # Errors
///
/// [`OmexError::Refused`] if the location is empty, absolute, starts with a
/// drive letter or has a `..` segment.
pub fn normalize_location(location: &str) -> Result<String, OmexError> {
    if location.is_empty() {
        return Err(OmexError::Refused(
            "location of an archive entry must not be empty".to_string(),
        ));
    }
    let path = location.replace('\\', "/");
    let mut chars = location.chars();
    let drive = matches!(
        (chars.next(), chars.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    );
    if path.starts_with('/') || drive {
        return Err(OmexError::Refused(format!(
            "locations must be relative paths, but location is '{location}'"
        )));
    }
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                return Err(OmexError::Refused(format!(
                    "locations must not point outside of the archive, but location is '{location}'"
                )));
            }
            segment => segments.push(segment),
        }
    }
    if segments.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(format!("./{}", segments.join("/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain relative path gets the `./` prefix.
    #[test]
    fn normalizes_plain_relative() {
        assert_eq!(
            normalize_location("models/model.xml").unwrap(),
            "./models/model.xml"
        );
    }
    /// Backslashes become slashes.
    #[test]
    fn normalizes_backslashes() {
        assert_eq!(
            normalize_location(".\\models\\model.xml").unwrap(),
            "./models/model.xml"
        );
    }
    /// An existing `./` prefix is kept.
    #[test]
    fn keeps_dot_prefix() {
        assert_eq!(
            normalize_location("./manifest.xml").unwrap(),
            "./manifest.xml"
        );
    }
    /// The archive itself is `.`.
    #[test]
    fn archive_itself_is_dot() {
        assert_eq!(normalize_location(".").unwrap(), ".");
    }
    /// Empty segments are dropped.
    #[test]
    fn drops_empty_segments() {
        assert_eq!(
            normalize_location("./models//model.xml").unwrap(),
            "./models/model.xml"
        );
    }
    /// An empty location is refused.
    #[test]
    fn refuses_empty() {
        assert!(normalize_location("").is_err());
    }
    /// An absolute location is refused.
    #[test]
    fn refuses_absolute() {
        assert!(normalize_location("/model.xml").is_err());
    }
    /// A drive letter is refused.
    #[test]
    fn refuses_drive_letter() {
        assert!(normalize_location("C:/model.xml").is_err());
    }
    /// A parent segment is refused.
    #[test]
    fn refuses_parent() {
        assert!(normalize_location("../model.xml").is_err());
    }
    /// A parent segment in the middle is refused.
    #[test]
    fn refuses_inner_parent() {
        assert!(normalize_location("a/../b").is_err());
    }
    /// A backslash parent segment is refused.
    #[test]
    fn refuses_backslash_parent() {
        assert!(normalize_location("..\\model.xml").is_err());
    }
}
