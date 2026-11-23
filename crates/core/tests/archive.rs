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
        archive.remove_entry("model.xml").unwrap().unwrap().location,
        "./model.xml"
    );
    assert!(archive.entry("./model.xml").is_none());
    assert!(matches!(
        archive.entry_bytes("./model.xml"),
        Err(OmexError::NotFound(_))
    ));
    assert!(archive.remove_entry("./model.xml").unwrap().is_none());
}

/// `.` and the manifest stay.
#[test]
fn remove_entry_refuses_the_required_entries() {
    let mut archive = hand_built();
    let before = archive.manifest().clone();
    assert!(archive.remove_entry(".").unwrap().is_none());
    assert!(archive.remove_entry("./manifest.xml").unwrap().is_none());
    assert_eq!(archive.manifest(), &before);
}

/// A location no archive can hold is refused, as it is by every other call which takes one.
#[test]
fn remove_entry_refuses_a_location_outside_of_the_archive() {
    let mut archive = hand_built();
    let before = archive.manifest().clone();
    for location in ["", "/abs.xml", "C:/x.xml", "../x.xml", "a/../b"] {
        assert!(
            matches!(archive.remove_entry(location), Err(OmexError::Refused(_))),
            "{location}"
        );
    }
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
    assert!(archive.remove_entry("./extra.txt").unwrap().is_some());
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
    assert!(archive.remove_entry("./extra.txt").unwrap().is_some());
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

/// An encrypted archive opens with its password and is refused without one.
///
/// `scripts/zip_edge_fixtures.py` builds the fixture, its manifest included, under the
/// password `test1234`.
#[test]
fn an_encrypted_archive_needs_its_password() {
    let path = format!(
        "{}/tests/data/zip/encrypted_omex.zip",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap();
    // a password which is missing and one which is wrong are their own kind of refusal, so
    // that a host can tell a member it may not read from a container which has no manifest
    assert!(matches!(
        Archive::from_bytes(&bytes),
        Err(OmexError::Password(_))
    ));
    assert!(matches!(
        Archive::from_bytes_with_password(&bytes, b"wrong"),
        Err(OmexError::Password(_))
    ));
    assert_eq!(
        pymetadata_core::wire::kind(&Archive::from_bytes(&bytes).unwrap_err()),
        "password"
    );
    assert!(
        Archive::from_bytes(&bytes)
            .unwrap_err()
            .to_string()
            .starts_with("refused: "),
        "the message is the one hosts have been reading"
    );
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

#[test]
fn members_are_the_zip_members_in_order_with_their_sizes() {
    let archive = Archive::from_bytes(&fixture("iCGB21FR.omex")).unwrap();
    let members = archive.members();
    assert_eq!(members.len(), 22);
    assert_eq!(members[0].location, "./manifest.xml");
    assert!(members.iter().all(|m| m.location.starts_with("./")));
    assert!(
        members
            .iter()
            .any(|m| m.location == "./__MACOSX/._manifest.xml")
    );
    assert!(
        members
            .iter()
            .filter(|m| m.method == 8)
            .all(|m| m.compressed_size > 0)
    );
}

#[test]
fn members_of_an_archive_built_from_nothing_are_empty() {
    assert!(Archive::new().members().is_empty());
}

#[test]
fn has_bytes_is_false_for_a_manifest_entry_without_a_member() {
    let archive = Archive::from_bytes(&fixture("iCGB21FR.omex")).unwrap();
    assert!(archive.entry("./iCGB21FR_memote.html").is_some());
    assert!(!archive.has_bytes("./iCGB21FR_memote.html").unwrap());
    assert!(archive.has_bytes("iCGB21FR.xml").unwrap());
}

#[test]
fn has_bytes_is_true_after_set_entry_on_an_archive_built_from_nothing() {
    let mut archive = Archive::new();
    let entry = Entry {
        location: "./a.txt".to_string(),
        format: guess_format("txt", &[]).to_string(),
        master: false,
    };
    archive.set_entry(entry, b"x".to_vec()).unwrap();
    assert!(archive.has_bytes("a.txt").unwrap());
    assert!(archive.members().is_empty());
}

#[test]
fn has_bytes_refuses_a_location_outside_of_the_archive() {
    assert!(matches!(
        Archive::new().has_bytes("../x"),
        Err(OmexError::Refused(_))
    ));
}

#[test]
fn has_bytes_is_false_for_the_derived_locations() {
    let archive = Archive::from_bytes(&fixture("iCGB21FR.omex")).unwrap();
    assert!(!archive.has_bytes(".").unwrap());
    assert!(!archive.has_bytes("./manifest.xml").unwrap());
}

#[test]
fn a_duplicate_member_name_appears_once_per_member() {
    let bytes = stored_zip(&[
        ("manifest.xml", MANIFEST),
        ("a.txt", b"one"),
        ("a.txt", b"two"),
    ]);
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    let members = archive.members();
    let a: Vec<_> = members.iter().filter(|m| m.location == "./a.txt").collect();
    assert_eq!(a.len(), 2);
    assert!(archive.has_bytes("a.txt").unwrap());
    assert_eq!(archive.entry_bytes("a.txt").unwrap(), b"two");
}

#[test]
fn has_bytes_follows_remove_entry_while_members_does_not_shrink() {
    let mut archive = Archive::from_bytes(&fixture("iCGB21FR.omex")).unwrap();
    let before = archive.members();
    assert!(archive.has_bytes("./iCGB21FR.xml").unwrap());
    archive.remove_entry("./iCGB21FR.xml").unwrap();
    assert!(!archive.has_bytes("./iCGB21FR.xml").unwrap());
    assert_eq!(archive.members(), before);
    // a member the manifest did not list was added to it on opening, so it is the same
    let unlisted = "./__MACOSX/._manifest.xml";
    assert!(archive.has_bytes(unlisted).unwrap());
    archive.remove_entry(unlisted).unwrap();
    assert!(!archive.has_bytes(unlisted).unwrap());
}

#[test]
fn declared_entries_are_the_manifest_of_the_container_without_the_invented_ones() {
    let bytes = fixture("iCGB21FR.omex");
    let archive = Archive::from_bytes(&bytes).unwrap();
    let declared: Vec<&str> = archive
        .declared_entries()
        .iter()
        .map(|e| e.location.as_str())
        .collect();
    // the expectation is read from the manifest text with a crude split, deliberately not with
    // `Manifest::parse`, which would check the parser against itself
    let zip = zip_manifest(&bytes);
    let expected: Vec<String> = zip
        .split("location=\"")
        .skip(1)
        .map(|rest| {
            let location = rest.split('"').next().unwrap();
            pymetadata_core::normalize_location(location).unwrap()
        })
        .collect();
    assert_eq!(declared, expected);
    assert!(declared.contains(&"./iCGB21FR_memote.html"));
    assert!(!declared.contains(&"./__MACOSX/._manifest.xml"));
    assert!(declared.len() < archive.manifest().entries().len());
}

/// The text of the `manifest.xml` of an archive, read through the zip reader.
fn zip_manifest(bytes: &[u8]) -> String {
    let reader = pymetadata_core::zip::Reader::new(bytes.to_vec()).unwrap();
    let member = reader
        .members()
        .iter()
        .find(|m| m.name == "manifest.xml")
        .unwrap();
    String::from_utf8(reader.inflate(member).unwrap()).unwrap()
}

#[test]
fn declared_entries_do_not_prepend_the_required_entries() {
    let archive = Archive::from_bytes(&fixture("BIOMD0000000001.omex")).unwrap();
    let declared = archive.declared_entries();
    assert!(
        declared
            .iter()
            .all(|e| e.location != "." && e.location != "./manifest.xml")
    );
    let text = zip_manifest(&fixture("BIOMD0000000001.omex"));
    assert_eq!(declared.len(), text.matches("<content ").count());
    let entries = archive.manifest().entries();
    assert!(entries.iter().any(|e| e.location == "."));
    assert!(entries.iter().any(|e| e.location == "./manifest.xml"));
}

#[test]
fn declared_entries_are_unchanged_by_set_entry_and_remove_entry() {
    let mut archive = Archive::from_bytes(&fixture("iCGB21FR.omex")).unwrap();
    let before = archive.declared_entries().to_vec();
    let entry = Entry {
        location: "./new.txt".to_string(),
        format: guess_format("txt", &[]).to_string(),
        master: false,
    };
    archive.set_entry(entry, b"x".to_vec()).unwrap();
    archive.remove_entry("./iCGB21FR.xml").unwrap();
    assert_eq!(archive.declared_entries(), before.as_slice());
}

#[test]
fn declared_entries_of_an_archive_built_from_nothing_are_empty() {
    assert!(Archive::new().declared_entries().is_empty());
}

#[test]
fn a_location_declared_twice_is_declared_twice_and_listed_once() {
    let manifest = br#"<?xml version="1.0"?>
<omexManifest xmlns="http://identifiers.org/combine.specifications/omex-manifest">
  <content location="./model.xml" format="http://identifiers.org/combine.specifications/sbml" master="false"/>
  <content location="model.xml" format="http://identifiers.org/combine.specifications/sbml" master="true"/>
</omexManifest>"#;
    let bytes = stored_zip(&[("manifest.xml", manifest), ("model.xml", b"<sbml/>")]);
    let archive = Archive::from_bytes(&bytes).unwrap();
    assert_eq!(archive.declared_entries().len(), 2);
    let effective: Vec<_> = archive
        .manifest()
        .entries()
        .iter()
        .filter(|e| e.location == "./model.xml")
        .collect();
    assert_eq!(effective.len(), 1);
    assert!(effective[0].master);
}

/// A metadata only change leaves the member of the entry the compressed blob it was.
#[test]
fn set_entry_format_does_not_recompress_the_member() {
    let bytes = fixture("CompModels.omex");
    let source = Archive::from_bytes(&bytes).unwrap();
    let before: Vec<_> = source.members().to_vec();
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    let target = archive
        .locations()
        .find(|location| location.ends_with(".xml") && *location != "./manifest.xml")
        .unwrap()
        .to_string();
    archive
        .set_entry_format(
            &target,
            "http://purl.org/NET/mediatypes/application/xml",
            true,
        )
        .unwrap();
    let entry = archive.entry(&target).unwrap();
    assert_eq!(
        entry.format,
        "http://purl.org/NET/mediatypes/application/xml"
    );
    assert!(entry.master);
    let written = archive.to_bytes().unwrap();
    let result = Archive::from_bytes(&written).unwrap();
    for member in &before {
        if member.location == "./manifest.xml" {
            continue;
        }
        let after = result
            .members()
            .iter()
            .find(|candidate| candidate.location == member.location)
            .unwrap_or_else(|| panic!("member {} is gone", member.location))
            .clone();
        assert_eq!(
            (after.method, after.compressed_size),
            (member.method, member.compressed_size),
            "member {} was recompressed",
            member.location
        );
    }
    assert_eq!(
        result.entry(&target).unwrap().format,
        "http://purl.org/NET/mediatypes/application/xml",
        "the written manifest carries the new format"
    );
    assert!(result.entry(&target).unwrap().master);
}

/// The entry keeps the place it had in the manifest.
#[test]
fn set_entry_format_keeps_the_position_of_the_entry() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    let before: Vec<String> = archive.locations().map(str::to_string).collect();
    let target = before
        .iter()
        .find(|location| location.ends_with(".xml") && *location != "./manifest.xml")
        .unwrap()
        .clone();
    archive
        .set_entry_format(&target, "text/plain", false)
        .unwrap();
    let after: Vec<String> = archive.locations().map(str::to_string).collect();
    assert_eq!(before, after);
}

/// The two entries the specification prescribes are not changed through this call.
#[test]
fn set_entry_format_refuses_the_derived_locations() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    for location in [".", "./manifest.xml", "manifest.xml"] {
        let error = archive.set_entry_format(location, "text/plain", false);
        assert!(
            matches!(error, Err(OmexError::Refused(_))),
            "{location}: {error:?}"
        );
    }
}

/// A location the manifest does not list is not added by a metadata change.
#[test]
fn set_entry_format_reports_an_unknown_location_as_not_found() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    let error = archive.set_entry_format("./nothing.xml", "text/plain", false);
    assert!(matches!(error, Err(OmexError::NotFound(_))), "{error:?}");
    assert!(archive.entry("./nothing.xml").is_none());
}

/// The order a host sets is the order the written archive and its manifest carry.
#[test]
fn reorder_entries_survives_a_round_trip() {
    let mut archive = Archive::from_bytes(&fixture("iCGB21FR.omex")).unwrap();
    let mut wanted: Vec<String> = archive
        .locations()
        .filter(|location| *location != "." && *location != "./manifest.xml")
        .map(str::to_string)
        .collect();
    wanted.sort();
    assert_ne!(
        wanted,
        archive
            .locations()
            .filter(|l| *l != "." && *l != "./manifest.xml")
            .map(str::to_string)
            .collect::<Vec<_>>(),
        "the fixture is worth the test only if the orders differ"
    );
    archive.reorder_entries(&wanted).unwrap();
    let written = archive.to_bytes().unwrap();
    let result = Archive::from_bytes(&written).unwrap();
    let after: Vec<String> = result
        .locations()
        .filter(|location| *location != "." && *location != "./manifest.xml")
        .map(str::to_string)
        .collect();
    assert_eq!(after, wanted);
    assert_eq!(
        result.declared_entries().len(),
        archive.manifest().entries().len(),
        "every entry is declared, in that order"
    );
}

/// Naming a few entries sorts those few, into the slots they held, and moves nothing else.
#[test]
fn reorder_entries_permutes_the_named_entries_among_their_slots() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    let before: Vec<String> = archive.locations().map(str::to_string).collect();
    let first = before[2].clone();
    let last = before[before.len() - 1].clone();
    archive
        .reorder_entries(&[last.clone(), first.clone()])
        .unwrap();
    let after: Vec<String> = archive.locations().map(str::to_string).collect();
    assert_eq!(after[2], last, "the named pair swapped");
    assert_eq!(after[after.len() - 1], first);
    for index in 0..before.len() {
        if index != 2 && index != before.len() - 1 {
            assert_eq!(after[index], before[index], "entry {index} moved");
        }
    }
}

/// An order which names an unknown location, or one twice, changes nothing.
#[test]
fn reorder_entries_validates_before_it_moves() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    let before: Vec<String> = archive.locations().map(str::to_string).collect();
    let unknown = archive.reorder_entries(&["./nothing.xml"]);
    assert!(
        matches!(unknown, Err(OmexError::NotFound(_))),
        "{unknown:?}"
    );
    let twice = archive.reorder_entries(&[before[2].clone(), before[2].clone()]);
    assert!(matches!(twice, Err(OmexError::Refused(_))), "{twice:?}");
    let outside = archive.reorder_entries(&["../evil.txt"]);
    assert!(matches!(outside, Err(OmexError::Refused(_))), "{outside:?}");
    assert_eq!(
        archive.locations().map(str::to_string).collect::<Vec<_>>(),
        before,
        "a refused order moves nothing"
    );
}

/// A head is the prefix of the member, and it inflates no more than it returns.
#[test]
fn head_reads_the_first_bytes_of_a_member() {
    for name in ARCHIVES {
        let bytes = fixture(name);
        let mut archive = Archive::from_bytes(&bytes).unwrap();
        let locations: Vec<String> = archive
            .locations()
            .filter(|location| *location != "." && *location != "./manifest.xml")
            .map(str::to_string)
            .collect();
        for location in locations {
            if !archive.has_bytes(&location).unwrap() {
                continue;
            }
            let Ok(head) = archive.head(&location, 256) else {
                continue; // a member this core cannot read has no head either
            };
            let whole = archive.entry_bytes(&location).unwrap().to_vec();
            assert_eq!(head, whole[..whole.len().min(256)], "{name}: {location}");
            assert!(head.len() <= 256);
        }
    }
}

/// A head of an entry whose bytes are at hand comes from those bytes.
#[test]
fn head_serves_what_is_already_read_and_what_the_caller_set() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    archive
        .set_entry(
            Entry::new("./new.txt", "text/plain", false).unwrap(),
            b"0123456789".to_vec(),
        )
        .unwrap();
    assert_eq!(archive.head("./new.txt", 4).unwrap(), b"0123");
    assert_eq!(archive.head("./new.txt", 99).unwrap(), b"0123456789");
    let location = archive
        .locations()
        .find(|l| l.ends_with(".xml") && *l != "./manifest.xml")
        .unwrap()
        .to_string();
    let whole = archive.entry_bytes(&location).unwrap().to_vec();
    assert_eq!(archive.head(&location, 8).unwrap(), whole[..8]);
}

/// A limit of zero reads nothing, and a member shorter than the limit gives what it holds.
#[test]
fn head_is_bounded_by_the_limit_and_by_the_member() {
    let zip = stored_zip(&[("manifest.xml", MANIFEST), ("short.txt", b"ab")]);
    let mut archive = Archive::from_bytes(&zip).unwrap();
    assert_eq!(archive.head("./short.txt", 0).unwrap(), b"");
    assert_eq!(archive.head("./short.txt", 1).unwrap(), b"a");
    assert_eq!(archive.head("./short.txt", 256).unwrap(), b"ab");
}

/// A head answers for a member the budget of the reader refuses to inflate whole.
#[test]
fn head_reads_a_member_over_the_inflate_budget() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    archive.set_max_inflated_size(64);
    let location = archive
        .locations()
        .find(|l| l.ends_with(".xml") && *l != "./manifest.xml")
        .unwrap()
        .to_string();
    let head = archive.head(&location, 16).unwrap();
    assert_eq!(
        head.len(),
        16,
        "the head is bounded by its limit, not the budget"
    );
    let whole = archive.entry_bytes(&location);
    assert!(
        matches!(whole, Err(OmexError::Refused(_))),
        "the whole member is still over the budget: {whole:?}"
    );
}

/// The derived locations and an unknown one answer as `entry_bytes` does.
#[test]
fn head_refuses_the_manifest_and_reports_an_unknown_location() {
    let mut archive = Archive::from_bytes(&fixture("CompModels.omex")).unwrap();
    assert!(matches!(
        archive.head("./manifest.xml", 8),
        Err(OmexError::Refused(_))
    ));
    assert!(matches!(archive.head(".", 8), Err(OmexError::NotFound(_))));
    assert!(matches!(
        archive.head("./nothing.xml", 8),
        Err(OmexError::NotFound(_))
    ));
    assert!(matches!(
        archive.head("../evil.txt", 8),
        Err(OmexError::Refused(_))
    ));
}
