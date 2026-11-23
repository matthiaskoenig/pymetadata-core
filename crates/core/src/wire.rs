//! The JSON wire types of the hosts.
//!
//! The wasm host crosses its boundary with a raw C ABI where every structured value is a JSON
//! string, and the python binding reports an error by the same kind strings. The envelopes are
//! decided here. The entries inside them are [`Entry`] itself, which derives `Serialize` and
//! `Deserialize`, so the field names of `Entry` are part of the wire contract and renaming one
//! changes the JSON the hosts read and write.

use serde::{Deserialize, Serialize};

use crate::error::OmexError;
use crate::manifest::Entry;

/// The answer of a manifest request: the entries of the archive in document order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestResponse {
    /// The entries, each as `{"location": ..., "format": ..., "master": ...}`. These keys are the
    /// field names of [`Entry`], which is serialized as it is: do not rename them as if they were
    /// internal.
    pub entries: Vec<Entry>,
}

/// The JSON header of a request which sets an entry, followed on the wire by `len` bytes.
///
/// An unknown field is an error, so a misspelled `format` or `master` in a header written by
/// hand is reported and not silently replaced by its default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetEntryRequest {
    /// Location of the entry, normalized when the entry is built from it.
    pub location: String,
    /// Format URI of the entry, absent when the host leaves it to the core to guess it.
    #[serde(default)]
    pub format: Option<String>,
    /// Marks the entry a tool should open first, `false` when absent.
    #[serde(default)]
    pub master: bool,
    /// Number of content bytes which follow the header. It is a `usize`, so on `wasm32` a value
    /// above 4 GiB fails to deserialize, which is the bound of the target's pointer width.
    pub len: usize,
}

/// The answer of a failed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorResponse {
    /// What kind of failure it is, see [`kind`].
    pub kind: &'static str,
    /// A human readable description.
    pub message: String,
}

/// The stable kind string of an error, one per variant of [`OmexError`].
///
/// - `malformed`, `refused` and `unsupported` name the variant of the same name.
/// - `not_found` names [`OmexError::NotFound`], so a host can tell a missing entry from a broken
///   archive.
///
/// A fifth kind, `internal`, is not returned here: no [`OmexError`] is an internal failure, and
/// a host which fails for a reason outside of the core (no archive open, a panic) reports that
/// itself.
pub fn kind(error: &OmexError) -> &'static str {
    match error {
        OmexError::Malformed(_) => "malformed",
        OmexError::Refused(_) => "refused",
        OmexError::Unsupported(_) => "unsupported",
        OmexError::NotFound(_) => "not_found",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The response lists the entries in order under the keys `location`, `format`, `master`.
    #[test]
    fn manifest_response_serializes_entries_in_order() {
        let entries = ["./a.xml", "./b.xml", "./c.xml"]
            .iter()
            .enumerate()
            .map(|(i, location)| Entry::new(location, "http://example.org/f", i == 1).unwrap())
            .collect();
        let json = serde_json::to_string(&ManifestResponse { entries }).unwrap();
        assert_eq!(
            json,
            concat!(
                r#"{"entries":["#,
                r#"{"location":"./a.xml","format":"http://example.org/f","master":false},"#,
                r#"{"location":"./b.xml","format":"http://example.org/f","master":true},"#,
                r#"{"location":"./c.xml","format":"http://example.org/f","master":false}]}"#
            )
        );
    }

    /// A request without a format and without a master flag takes `None` and `false`.
    #[test]
    fn set_entry_request_parses_without_a_format() {
        let request: SetEntryRequest =
            serde_json::from_str(r#"{"location":"./a.xml","len":3}"#).unwrap();
        assert_eq!(
            request,
            SetEntryRequest {
                location: "./a.xml".to_string(),
                format: None,
                master: false,
                len: 3,
            }
        );
    }

    /// An explicit format and master flag are read, and `"format":null` is the same as absent.
    #[test]
    fn set_entry_request_parses_every_field() {
        let request: SetEntryRequest = serde_json::from_str(
            r#"{"location":"./a.xml","format":"http://example.org/f","master":true,"len":0}"#,
        )
        .unwrap();
        assert_eq!(request.format.as_deref(), Some("http://example.org/f"));
        assert!(request.master);
        let request: SetEntryRequest =
            serde_json::from_str(r#"{"location":"./a.xml","format":null,"len":1}"#).unwrap();
        assert_eq!(request.format, None);
    }

    /// An unknown field, e.g. a misspelled one, and a missing required field are rejected.
    #[test]
    fn set_entry_request_rejects_unknown_and_missing_fields() {
        for json in [
            r#"{"location":"./a.xml","len":3,"mastr":true}"#,
            r#"{"location":"./a.xml","len":3,"formt":"x"}"#,
            r#"{"len":3}"#,
            r#"{"location":"./a.xml"}"#,
        ] {
            assert!(
                serde_json::from_str::<SetEntryRequest>(json).is_err(),
                "{json}"
            );
        }
    }

    /// Every variant has its own kind string, and the error response carries it.
    #[test]
    fn error_kinds_are_stable() {
        let cases = [
            (OmexError::Malformed("m".into()), "malformed"),
            (OmexError::Refused("r".into()), "refused"),
            (OmexError::Unsupported("u".into()), "unsupported"),
            (OmexError::NotFound("n".into()), "not_found"),
        ];
        for (error, expected) in &cases {
            assert_eq!(kind(error), *expected);
        }
        let response = ErrorResponse {
            kind: kind(&cases[3].0),
            message: "gone".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"kind":"not_found","message":"gone"}"#
        );
    }
}
