//! Parity of the format table with `pymetadata.omex`.

use pymetadata_core::{UNKNOWN_FORMAT, guess_format, lookup_format};

/// Every key of `pymetadata.omex.EntryFormat` resolves to the same URI.
#[test]
fn every_key_of_pymetadata_resolves_to_the_same_uri() {
    let text = include_str!("data/formats.json");
    let pairs: std::collections::BTreeMap<String, String> = serde_json::from_str(text).unwrap();
    assert!(pairs.len() > 100);
    for (key, uri) in &pairs {
        assert_eq!(lookup_format(key), uri, "key {key}");
        assert_eq!(lookup_format(&key.to_uppercase()), uri, "key {key}");
    }
}

/// An unknown key is the unknown media type.
#[test]
fn unknown_key_is_the_unknown_media_type() {
    assert_eq!(
        lookup_format("nosuchformat"),
        "https://purl.org/NET/mediatypes/application/x.unknown"
    );
    assert_eq!(UNKNOWN_FORMAT, lookup_format("nosuchformat"));
}

/// The root tag of an `.xml` file decides.
#[test]
fn guesses_sbml_from_the_root_tag() {
    // asserts the generic URI because the root tag table maps `<sbml` to the generic key `sbml`
    assert_eq!(
        guess_format(
            "xml",
            b"<?xml version='1.0'?><sbml xmlns='http://www.sbml.org/sbml/level3/version2/core'>"
        ),
        "http://identifiers.org/combine.specifications/sbml"
    );
}

/// Without a root tag the suffix decides, whatever its case.
#[test]
fn guesses_xml_and_csv_from_the_suffix() {
    assert_eq!(
        guess_format("XML", b"<other/>"),
        "https://purl.org/NET/mediatypes/application/xml"
    );
    assert_eq!(
        guess_format("csv", b""),
        "https://purl.org/NET/mediatypes/text/csv"
    );
}

/// An unknown suffix is the unknown media type.
#[test]
fn guesses_unknown_for_an_unknown_suffix() {
    assert_eq!(
        guess_format("zzz", b""),
        "https://purl.org/NET/mediatypes/application/x.unknown"
    );
}

/// The `sedml`, `cellml` and `copasi` root tags give the URI python returns for their key.
#[test]
fn guesses_the_other_root_tags() {
    for (tag, key) in [
        (&b"<sedML level='1'>"[..], "sedml"),
        (&b"<cell xmlns='x'>"[..], "cellml"),
        (&b"<COPASI xmlns='x'>"[..], "copasi"),
    ] {
        assert_eq!(guess_format("xml", tag), lookup_format(key), "tag {tag:?}");
        assert_ne!(lookup_format(key), UNKNOWN_FORMAT, "key {key}");
    }
}

/// With two tags in the head the first one of the table wins, not the first in the file.
#[test]
fn first_match_of_the_table_wins() {
    assert_eq!(
        guess_format("xml", b"<COPASI><sedML><sbml>"),
        lookup_format("sbml")
    );
    assert_eq!(
        guess_format("xml", b"<COPASI><sedML>"),
        lookup_format("sedml")
    );
}

/// A tag beyond the first 256 bytes is not found, so the suffix decides.
#[test]
fn tag_beyond_the_head_is_not_found() {
    let mut head = vec![b' '; 256];
    head.extend_from_slice(b"<sbml>");
    assert_eq!(guess_format("xml", &head), lookup_format("xml"));
    let mut head = vec![b' '; 250];
    head.extend_from_slice(b"<sbml>");
    assert_eq!(guess_format("xml", &head), lookup_format("sbml"));
}
