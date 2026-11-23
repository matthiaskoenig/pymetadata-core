//! Parity of the manifest with `pymetadata.omex`.
//!
//! `scripts/manifest_parity.py` writes the `manifest.xml` of each archive of the pymetadata
//! test data and the entries `Omex.from_omex` reads from it.

use std::collections::BTreeMap;

use pymetadata_core::{Entry, Manifest};

/// The `manifest.xml` of each archive parses to the entries of pymetadata, in the same order,
/// after at most the two required entries.
#[test]
fn manifests_parse_to_the_entries_of_pymetadata() {
    let text = include_str!("data/manifests.json");
    let expected: BTreeMap<String, Vec<Entry>> = serde_json::from_str(text).unwrap();
    assert_eq!(expected.len(), 4);
    for (archive, entries) in &expected {
        let stem = archive.strip_suffix(".omex").unwrap();
        let path = format!(
            "{}/tests/data/manifests/{stem}.xml",
            env!("CARGO_MANIFEST_DIR")
        );
        let xml = std::fs::read_to_string(&path).unwrap();
        let manifest = Manifest::parse(&xml).unwrap();
        // `Manifest.from_manifest` of pymetadata lists what the file lists, the core puts the
        // required entries which the file lacks in front (pymetadata adds them elsewhere)
        let (lead, rest) = manifest
            .entries()
            .split_at(manifest.entries().len() - entries.len());
        assert_eq!(rest, entries.as_slice(), "archive {archive}");
        for entry in lead {
            assert!(
                matches!(entry.location.as_str(), "." | "./manifest.xml"),
                "archive {archive}"
            );
        }
        // the serialized manifest parses back to the same entries
        assert_eq!(
            Manifest::parse(&manifest.to_xml()).unwrap(),
            manifest,
            "archive {archive}"
        );
    }
}
