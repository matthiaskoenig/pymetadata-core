//! The lazy archive, against the archive corpus and hand built archives.

use pymetadata_core::{Archive, Entry, OmexError, guess_format};

/// The archives of the corpus.
const ARCHIVES: [&str; 4] = [
    "BIOMD0000000001.omex",
    "CombineArchiveShowCase.omex",
    "CompModels.omex",
    "iCGB21FR.omex",
];

/// Read an archive of the corpus.
fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/omex/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {path}: {error}"))
}

/// Build a zip archive of stored members, which is all these tests need.
fn stored_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(data);
        let size = data.len() as u32;
        let name_length = name.len() as u16;
        let mut common = Vec::new();
        common.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, stored, time, date
        common.extend_from_slice(&crc.to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes());
        common.extend_from_slice(&name_length.to_le_bytes());
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&common);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02\x14\x00");
        central.extend_from_slice(&common);
        central.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// The manifest of the hand built archives, which lists `./model.xml` only.
const MANIFEST: &[u8] = br#"<?xml version="1.0"?>
<omexManifest xmlns="http://identifiers.org/combine.specifications/omex-manifest">
  <content location="." format="http://identifiers.org/combine.specifications/omex"/>
  <content location="./manifest.xml" format="http://identifiers.org/combine.specifications/omex-manifest"/>
  <content location="./model.xml" format="http://identifiers.org/combine.specifications/sbml" master="true"/>
</omexManifest>"#;

/// An archive whose members are `manifest.xml`, `model.xml` and `extra.txt`.
fn hand_built() -> Archive {
    let bytes = stored_zip(&[
        ("manifest.xml", MANIFEST),
        ("model.xml", b"<sbml/>"),
        ("extra.txt", b"extra"),
    ]);
    Archive::from_bytes(&bytes).unwrap()
}

/// Every fixture opens and lists its entries, the two required ones included.
#[test]
fn opens_every_fixture_and_lists_its_entries() {
    for name in ARCHIVES {
        let archive = Archive::from_bytes(&fixture(name)).unwrap();
        let entries = archive.manifest().entries();
        assert!(!entries.is_empty(), "{name}");
        assert!(archive.entry(".").is_some(), "{name}");
        assert!(archive.entry("./manifest.xml").is_some(), "{name}");
        assert_eq!(archive.locations().count(), entries.len(), "{name}");
    }
}

/// A member the manifest omits is listed, formatted by its suffix, and readable.
#[test]
fn lists_a_zip_member_absent_from_the_manifest() {
    let mut archive = hand_built();
    assert!(archive.locations().any(|l| l == "./extra.txt"));
    let entry = archive.entry("./extra.txt").unwrap();
    assert_eq!(entry.format, guess_format("txt", &[]));
    assert!(!entry.master);
    assert_eq!(archive.entry_bytes("./extra.txt").unwrap(), b"extra");
    assert!(archive.entry("./model.xml").unwrap().master);
}

/// The bytes of an entry read twice are the same.
#[test]
fn entry_bytes_reads_the_same_twice() {
    let mut archive = hand_built();
    let first = archive.entry_bytes("model.xml").unwrap().to_vec();
    assert_eq!(first, b"<sbml/>");
    assert_eq!(archive.entry_bytes("./model.xml").unwrap(), first);
}

/// A location which is not there is `NotFound`, an unlisted one included.
#[test]
fn entry_bytes_of_an_unknown_location_is_not_found() {
    let mut archive = hand_built();
    assert!(matches!(
        archive.entry_bytes("./nothing.xml"),
        Err(OmexError::NotFound(_))
    ));
    assert!(matches!(
        archive.entry_bytes("."),
        Err(OmexError::NotFound(_))
    ));
}

/// A new location is added and readable, and set again it holds the second bytes.
#[test]
fn set_entry_adds_and_replaces() {
    let mut archive = Archive::new();
    let entry = Entry::new("./a.txt", "text/plain", false).unwrap();
    archive.set_entry(entry.clone(), b"one".to_vec()).unwrap();
    assert_eq!(archive.entry("./a.txt"), Some(&entry));
    assert_eq!(archive.entry_bytes("./a.txt").unwrap(), b"one");
    archive.set_entry(entry, b"two".to_vec()).unwrap();
    assert_eq!(archive.entry_bytes("./a.txt").unwrap(), b"two");
    assert_eq!(archive.manifest().entries().len(), 3);
}

/// A replaced member of an opened archive reads as the replacement.
#[test]
fn set_entry_replaces_a_member() {
    let mut archive = hand_built();
    let entry = archive.entry("./model.xml").unwrap().clone();
    archive.set_entry(entry, b"<new/>".to_vec()).unwrap();
    assert_eq!(archive.entry_bytes("./model.xml").unwrap(), b"<new/>");
}

/// The location is normalized, in the manifest and for the content.
#[test]
fn set_entry_normalizes_the_location() {
    let mut archive = Archive::new();
    let entry = Entry {
        location: "models\\m.xml".to_string(),
        format: "x".to_string(),
        master: false,
    };
    archive.set_entry(entry, b"m".to_vec()).unwrap();
    assert!(archive.locations().any(|l| l == "./models/m.xml"));
    assert_eq!(archive.entry_bytes("models/m.xml").unwrap(), b"m");
}

/// A location which cannot be normalized changes nothing.
#[test]
fn set_entry_refuses_a_location_outside_of_the_archive() {
    let mut archive = Archive::new();
    let entry = Entry {
        location: "../m.xml".to_string(),
        format: "x".to_string(),
        master: false,
    };
    assert!(matches!(
        archive.set_entry(entry, Vec::new()),
        Err(OmexError::Refused(_))
    ));
    assert_eq!(archive.manifest().entries().len(), 2);
}

/// The bytes of `.` and of the manifest are not the caller's to set.
#[test]
fn set_entry_refuses_the_required_entries() {
    let mut archive = Archive::new();
    for location in [".", "./manifest.xml"] {
        let entry = Entry::new(location, "x", false).unwrap();
        assert!(matches!(
            archive.set_entry(entry, Vec::new()),
            Err(OmexError::Refused(_))
        ));
    }
}

/// A removed entry is gone from the manifest and from the content.
#[test]
fn remove_entry_removes_from_the_manifest_and_the_content() {
    let mut archive = hand_built();
    assert_eq!(
        archive.remove_entry("model.xml").unwrap().location,
        "./model.xml"
    );
    assert!(archive.entry("./model.xml").is_none());
    assert!(matches!(
        archive.entry_bytes("./model.xml"),
        Err(OmexError::NotFound(_))
    ));
    assert!(archive.remove_entry("./model.xml").is_none());
}

/// `.` and the manifest stay.
#[test]
fn remove_entry_refuses_the_required_entries() {
    let mut archive = hand_built();
    let before = archive.manifest().clone();
    assert!(archive.remove_entry(".").is_none());
    assert!(archive.remove_entry("./manifest.xml").is_none());
    assert_eq!(archive.manifest(), &before);
}

/// A new archive holds the two required entries.
#[test]
fn new_is_an_empty_archive_with_the_two_required_entries() {
    let archive = Archive::new();
    assert_eq!(archive.manifest().entries().len(), 2);
}

/// An archive without a manifest is malformed.
#[test]
fn an_archive_without_a_manifest_is_malformed() {
    let bytes = stored_zip(&[("model.xml", b"<sbml/>")]);
    assert!(matches!(
        Archive::from_bytes(&bytes),
        Err(OmexError::Malformed(_))
    ));
}

/// A member name pointing out of the archive is refused, a directory is skipped.
#[test]
fn member_names_are_checked() {
    let bad = stored_zip(&[("manifest.xml", MANIFEST), ("../evil.txt", b"x")]);
    assert!(matches!(
        Archive::from_bytes(&bad),
        Err(OmexError::Refused(_))
    ));
    let dir = stored_zip(&[("manifest.xml", MANIFEST), ("models/", b"")]);
    let archive = Archive::from_bytes(&dir).unwrap();
    assert!(archive.locations().all(|l| l != "./models"));
}

/// Garbage is an error and never a panic.
#[test]
fn garbage_is_malformed() {
    assert!(matches!(
        Archive::from_bytes(b"not a zip"),
        Err(OmexError::Malformed(_))
    ));
}

/// The bytes of the manifest are written from the entries and never served, however the
/// manifest changed since the archive was opened.
#[test]
fn entry_bytes_of_the_manifest_is_refused() {
    let mut archive = hand_built();
    assert!(matches!(
        archive.entry_bytes("./manifest.xml"),
        Err(OmexError::Refused(_))
    ));
    let entry = Entry::new("./new.txt", "text/plain", false).unwrap();
    archive.set_entry(entry, b"new".to_vec()).unwrap();
    archive.remove_entry("./extra.txt").unwrap();
    assert!(matches!(
        archive.entry_bytes("manifest.xml"),
        Err(OmexError::Refused(_))
    ));
}

/// A member which was inflated and is then replaced reads as the replacement.
#[test]
fn set_entry_replaces_an_inflated_member() {
    let mut archive = hand_built();
    assert_eq!(archive.entry_bytes("./extra.txt").unwrap(), b"extra");
    let entry = archive.entry("./extra.txt").unwrap().clone();
    archive.set_entry(entry, b"other".to_vec()).unwrap();
    assert_eq!(archive.entry_bytes("./extra.txt").unwrap(), b"other");
}

/// A member which was inflated and is then removed is gone.
#[test]
fn remove_entry_removes_an_inflated_member() {
    let mut archive = hand_built();
    archive.entry_bytes("./extra.txt").unwrap();
    archive.remove_entry("./extra.txt").unwrap();
    assert!(matches!(
        archive.entry_bytes("./extra.txt"),
        Err(OmexError::NotFound(_))
    ));
}

/// A member named `.` is the archive and holds no bytes.
#[test]
fn a_member_named_dot_is_skipped() {
    let bytes = stored_zip(&[("manifest.xml", MANIFEST), (".", b"x")]);
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    assert_eq!(archive.locations().filter(|l| *l == ".").count(), 1);
    assert!(matches!(
        archive.entry_bytes("."),
        Err(OmexError::NotFound(_))
    ));
}

/// The password of the archive reaches the reader: an unencrypted archive opens with one.
#[test]
fn from_bytes_with_password_opens_an_archive() {
    let bytes = stored_zip(&[("manifest.xml", MANIFEST), ("extra.txt", b"extra")]);
    let mut archive = Archive::from_bytes_with_password(&bytes, b"secret").unwrap();
    assert_eq!(archive.entry_bytes("./extra.txt").unwrap(), b"extra");
}

/// An encrypted archive (built by `scripts/zip_edge_fixtures.py`, manifest included, password `test1234`) opens with its password and is
/// refused without one.
#[test]
fn an_encrypted_archive_needs_its_password() {
    let path = format!(
        "{}/tests/data/zip/encrypted_omex.zip",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap();
    assert!(matches!(
        Archive::from_bytes(&bytes),
        Err(OmexError::Refused(_))
    ));
    assert!(Archive::from_bytes_with_password(&bytes, b"wrong").is_err());
    let mut archive = Archive::from_bytes_with_password(&bytes, b"test1234").unwrap();
    assert_eq!(archive.entry_bytes("./extra.txt").unwrap(), b"extra");
}

/// The allowance of the archive is the one of its reader.
#[test]
fn set_max_inflated_size_limits_a_member() {
    let bytes = stored_zip(&[("manifest.xml", MANIFEST), ("extra.txt", b"extra")]);
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    archive.set_max_inflated_size(2);
    assert!(matches!(
        archive.entry_bytes("./extra.txt"),
        Err(OmexError::Refused(_))
    ));
    archive.set_max_inflated_size(100);
    assert_eq!(archive.entry_bytes("./extra.txt").unwrap(), b"extra");
}
