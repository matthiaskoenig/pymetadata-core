//! The zip writer, against the archive corpus and hand built archives.
//!
//! The corpus is the four COMBINE archives of `tests/data/omex/`. The property the whole design
//! rests on is pinned here: a member the caller never touched is written as the compressed blob
//! it was read as, byte for byte, with its CRC32, its sizes and its method.
//!
//! What python's `zipfile` makes of the result is checked by `scripts/read_back.py`, which the
//! last test of this file writes its input for; the rust suite needs no python environment.

use std::collections::BTreeSet;

use pymetadata_core::zip::{Member, Reader, Writer};
use pymetadata_core::{Archive, Entry, OmexError, normalize_location};

/// The archives of the corpus.
const ARCHIVES: [&str; 4] = [
    "BIOMD0000000001.omex",
    "CombineArchiveShowCase.omex",
    "CompModels.omex",
    "iCGB21FR.omex",
];

/// The location of the archive itself, which is no member.
const ARCHIVE_LOCATION: &str = ".";
/// The location of the manifest, which is written from the entries.
const MANIFEST_LOCATION: &str = "./manifest.xml";
/// The stored method.
const STORED: u16 = 0;
/// The deflate method.
const DEFLATE: u16 = 8;
/// Bit 0 of the general purpose flags: the member is encrypted.
const ENCRYPTED: u16 = 0x0001;
/// Bit 3 of the general purpose flags: the sizes follow the data in a data descriptor.
const DATA_DESCRIPTOR: u16 = 0x0008;
/// The zip epoch, 1980-01-01 00:00:00, as an MS-DOS time and date.
const EPOCH: (u16, u16) = (0, (1 << 5) | 1);
/// The modification time `scripts/zip_edge_fixtures.py` stamps its hand built fixtures with,
/// 12:34:56 on 2026-01-01, which is no epoch and so tells a copy from a fresh member.
const STAMP: (u16, u16) = (0x645c, 0x5c21);
/// The external attributes of an executable file, which the members of `descriptors.zip` carry
/// and which no writer gives a member it compresses itself.
const EXECUTABLE: u32 = 0o100_755 << 16;
/// The external attributes of a member the writer compresses itself: a regular file, mode 644.
const REGULAR_FILE: u32 = 0o100_644 << 16;

/// The modification time of a member, as an MS-DOS time and date.
fn stamp_of(member: &Member) -> (u16, u16) {
    (member.dos_time, member.dos_date)
}

/// The modification time of the member named `name`, which the archive must hold.
fn stamp_in(reader: &Reader, name: &str) -> (u16, u16) {
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("the archive has no member {name:?}"));
    stamp_of(member)
}

/// Read an archive of the corpus.
fn fixture(relative: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{relative}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {path}: {error}"))
}

/// The name of the member which holds the entry of `location`.
fn member_name(location: &str) -> &str {
    location.strip_prefix("./").unwrap_or(location)
}

/// The locations of the members of `input`, as the archive names them.
fn locations_of(input: &Reader) -> BTreeSet<String> {
    input
        .members()
        .iter()
        // a directory is no entry of a COMBINE archive
        .filter(|member| !member.name.ends_with('/'))
        .map(|member| normalize_location(&member.name).unwrap())
        .collect()
}

/// The names of the members which `to_bytes` writes for `archive`, in the order of the manifest.
///
/// Every entry of the manifest the archive holds bytes for, plus the manifest, which is written
/// from those entries; `.` is no member, and an entry the zip does not hold has no bytes.
fn expected_members(archive: &Archive, input: &Reader) -> Vec<String> {
    let present = locations_of(input);
    archive
        .locations()
        .filter(|location| {
            *location == MANIFEST_LOCATION
                || (*location != ARCHIVE_LOCATION && present.contains(*location))
        })
        .map(|location| member_name(location).to_string())
        .collect()
}

/// Check that `written` holds the entries of `archive`, their bytes included.
fn assert_same_entries(archive: &mut Archive, written: &[u8], what: &str) {
    let mut read_back =
        Archive::from_bytes(written).unwrap_or_else(|error| panic!("{what}: {error}"));
    assert_eq!(
        read_back.manifest(),
        archive.manifest(),
        "{what}: the manifest of the written archive differs"
    );
    let locations: Vec<String> = archive.locations().map(str::to_string).collect();
    let mut compared = 0;
    for location in locations {
        if location == ARCHIVE_LOCATION || location == MANIFEST_LOCATION {
            continue;
        }
        let before = archive.entry_bytes(&location).map(<[u8]>::to_vec);
        let after = read_back.entry_bytes(&location).map(<[u8]>::to_vec);
        // the bytes are not in the message of a failure, there are megabytes of them
        assert!(
            before == after,
            "{what}: '{location}' is {} and was {}",
            outcome(&after),
            outcome(&before)
        );
        compared += 1;
    }
    assert!(compared > 0, "{what}: no entry was compared");
}

/// How many bytes an entry holds, or why it holds none, for the message of a failure.
fn outcome(bytes: &Result<Vec<u8>, OmexError>) -> String {
    match bytes {
        Ok(data) => format!("{} bytes", data.len()),
        Err(error) => error.to_string(),
    }
}

/// Every archive of the corpus is written and read back with the same entries and bytes.
#[test]
fn written_archive_reads_back_with_the_same_entries() {
    for name in ARCHIVES {
        let mut archive = Archive::from_bytes(&fixture(&format!("omex/{name}"))).unwrap();
        let written = archive.to_bytes().unwrap();
        assert_same_entries(&mut archive, &written, name);
    }
}

/// A member the caller never touched is copied as the blob it is, byte for byte.
#[test]
fn untouched_entries_are_copied_as_blobs() {
    for name in ARCHIVES {
        let bytes = fixture(&format!("omex/{name}"));
        let input = Reader::new(bytes.clone()).unwrap();
        let mut archive = Archive::from_bytes(&bytes).unwrap();
        // the write of "load, change one file, save": one entry is replaced, the rest is copied
        let replaced = expected_members(&archive, &input)
            .into_iter()
            .map(|name| normalize_location(&name).unwrap())
            .find(|location| location != MANIFEST_LOCATION)
            .unwrap();
        archive
            .set_entry(
                Entry::new(&replaced, "text/plain", false).unwrap(),
                b"replaced".to_vec(),
            )
            .unwrap();
        let output = Reader::new(archive.to_bytes().unwrap()).unwrap();
        let (mut copied, mut stamped) = (0, 0);
        for member in input.members() {
            // a directory is no entry of the archive, and the manifest is written from the
            // entries, so neither is copied
            if member.name.ends_with('/') {
                continue;
            }
            let location = normalize_location(&member.name).unwrap();
            if location == MANIFEST_LOCATION || location == replaced {
                continue;
            }
            let written = output
                .members()
                .iter()
                .find(|written| written.name == member_name(&location))
                .unwrap_or_else(|| panic!("{name}: '{location}' is not in the written archive"));
            assert_eq!(written.method, member.method, "{name}: {location} method");
            assert_eq!(written.crc32, member.crc32, "{name}: {location} crc32");
            assert_eq!(
                written.compressed_size, member.compressed_size,
                "{name}: {location} compressed size"
            );
            assert_eq!(
                written.uncompressed_size, member.uncompressed_size,
                "{name}: {location} uncompressed size"
            );
            assert_eq!(
                output.raw(written).unwrap(),
                input.raw(member).unwrap(),
                "{name}: the bytes of '{location}' were not copied"
            );
            // the sizes are written in the local header, so no member needs a data descriptor
            assert_eq!(
                written.flags & DATA_DESCRIPTOR,
                0,
                "{name}: {location} keeps bit 3"
            );
            // the modification time and the unix mode of the member are copied too
            assert_eq!(
                stamp_of(written),
                stamp_of(member),
                "{name}: {location} lost its modification time"
            );
            assert_eq!(
                written.external_attributes, member.external_attributes,
                "{name}: {location} lost its external attributes"
            );
            if stamp_of(member) != EPOCH {
                stamped += 1;
            }
            copied += 1;
        }
        assert!(copied > 0, "{name}: nothing was copied");
        assert!(
            stamped > 0,
            "{name}: no member of this archive has a modification time of its own"
        );
        // the entry which did change is the one which was written again
        let changed = output
            .members()
            .iter()
            .find(|member| member.name == member_name(&replaced))
            .unwrap_or_else(|| panic!("{name}: '{replaced}' is not in the written archive"));
        assert_eq!(output.inflate(changed).unwrap(), b"replaced", "{name}");
        assert_eq!(changed.method, DEFLATE, "{name}");
    }
}

/// A copied member keeps its unix mode, a member the writer compresses carries mode 644.
#[test]
fn a_copied_member_keeps_its_external_attributes() {
    // `scripts/zip_edge_fixtures.py` writes the members of `descriptors.zip` executable, a mode
    // the writer gives no member of its own, so a copy which stamped its own would show
    let input = Reader::new(fixture("zip/descriptors.zip")).unwrap();
    let member = input
        .members()
        .iter()
        .find(|member| member.name == "streamed.txt")
        .unwrap();
    assert_eq!(member.external_attributes, EXECUTABLE, "the fixture");
    let mut writer = Writer::new(Vec::new());
    writer
        .add_raw("streamed.txt", member, input.raw(member).unwrap())
        .unwrap();
    writer.add_stored("stored.txt", b"stored").unwrap();
    writer.add_deflated("deflated.txt", b"deflated", 6).unwrap();
    let output = Reader::new(writer.into_bytes().unwrap()).unwrap();
    let attributes: Vec<u32> = output
        .members()
        .iter()
        .map(|member| member.external_attributes)
        .collect();
    assert_eq!(attributes, [EXECUTABLE, REGULAR_FILE, REGULAR_FILE]);
}

/// A copied member keeps its modification time, a new or a replaced one carries the epoch.
#[test]
fn a_copied_member_keeps_its_modification_time() {
    let bytes = hand_built_zip(&[
        Raw::new("manifest.xml", MANIFEST),
        Raw::new("secret.txt", b"copied as it is"),
    ]);
    let input = Reader::new(bytes.clone()).unwrap();
    assert_eq!(stamp_in(&input, "secret.txt"), STAMP, "the fixture");
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    archive
        .set_entry(
            Entry::new("./new.txt", "text/plain", false).unwrap(),
            b"new".to_vec(),
        )
        .unwrap();
    let output = Reader::new(archive.to_bytes().unwrap()).unwrap();
    assert_eq!(stamp_in(&output, "secret.txt"), STAMP, "the copied member");
    assert_eq!(stamp_in(&output, "new.txt"), EPOCH, "the new member");
    assert_eq!(stamp_in(&output, "manifest.xml"), EPOCH, "the manifest");
    // recompressing is no copy: every member is inflated and deflated again
    let again = Reader::new(archive.to_bytes_recompressed(6).unwrap()).unwrap();
    for member in again.members() {
        assert_eq!(stamp_of(member), EPOCH, "{:?}", member.name);
    }
}

/// An archive behind a self extracting stub is written without it, with its timestamps.
///
/// `prepended.zip` is `CompModels.omex` behind a stub, so every offset of its central directory
/// is off by the length of the stub, and the archive it writes is the plain one.
#[test]
fn a_prepended_archive_is_written_without_its_stub() {
    let bytes = fixture("zip/prepended.zip");
    assert!(!bytes.starts_with(b"PK\x03\x04"), "the fixture has no stub");
    let input = Reader::new(bytes.clone()).unwrap();
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    let written = archive.to_bytes().unwrap();
    assert!(
        written.starts_with(b"PK\x03\x04"),
        "the archive was written behind something"
    );
    let output = Reader::new(written.clone()).unwrap();
    // the members of `CompModels.omex` carry 00:07:58 on 2021-10-06 and 16:31:18 on 2021-10-07
    assert_eq!(stamp_in(&output, "models/omex_comp.xml"), (0x00fd, 0x5346));
    assert_eq!(stamp_in(&output, "README.md"), (0x83e9, 0x5347));
    for member in input.members() {
        if member.name.ends_with('/') || member.name == "manifest.xml" {
            continue;
        }
        assert_eq!(
            stamp_in(&output, &member.name),
            stamp_of(member),
            "{:?} lost its modification time",
            member.name
        );
    }
    assert_same_entries(&mut archive, &written, "prepended.zip");
}

/// A member whose bytes a caller asked for is untouched and is copied as a blob all the same.
#[test]
fn a_member_which_was_read_is_still_copied_as_a_blob() {
    let bytes = fixture("omex/CompModels.omex");
    let input = Reader::new(bytes.clone()).unwrap();
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    let location = "./models/omex_comp.xml";
    archive.entry_bytes(location).unwrap();
    let output = Reader::new(archive.to_bytes().unwrap()).unwrap();
    let before = input
        .members()
        .iter()
        .find(|member| member.name == member_name(location))
        .unwrap();
    let after = output
        .members()
        .iter()
        .find(|member| member.name == member_name(location))
        .unwrap();
    assert_eq!(after.crc32, before.crc32);
    assert_eq!(after.compressed_size, before.compressed_size);
    assert_eq!(output.raw(after).unwrap(), input.raw(before).unwrap());
}

/// The manifest of the written archive is the one of the entries, and not the one read.
#[test]
fn the_manifest_is_regenerated() {
    let mut archive = Archive::from_bytes(&fixture("omex/CompModels.omex")).unwrap();
    archive
        .set_entry(
            Entry::new("./new/added.txt", "text/plain", true).unwrap(),
            b"added".to_vec(),
        )
        .unwrap();
    assert!(archive.remove_entry("./README.md").unwrap().is_some());
    let written = archive.to_bytes().unwrap();
    let reader = Reader::new(written.clone()).unwrap();
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == "manifest.xml")
        .unwrap();
    let xml = String::from_utf8(reader.inflate(member).unwrap()).unwrap();
    assert!(xml.contains("location=\"./new/added.txt\""), "{xml}");
    assert!(xml.contains("master=\"true\""), "{xml}");
    assert!(!xml.contains("README.md"), "{xml}");
    let mut read_back = Archive::from_bytes(&written).unwrap();
    assert_eq!(read_back.entry_bytes("./new/added.txt").unwrap(), b"added");
    assert!(read_back.entry("./README.md").is_none());
}

/// The members are written in the order of the manifest, and `.` is no member.
#[test]
fn the_members_are_written_in_the_order_of_the_manifest() {
    for name in ARCHIVES {
        let bytes = fixture(&format!("omex/{name}"));
        let input = Reader::new(bytes.clone()).unwrap();
        let mut archive = Archive::from_bytes(&bytes).unwrap();
        let expected = expected_members(&archive, &input);
        let reader = Reader::new(archive.to_bytes().unwrap()).unwrap();
        let written: Vec<String> = reader
            .members()
            .iter()
            .map(|member| member.name.clone())
            .collect();
        assert_eq!(written, expected, "{name}");
        assert!(written.contains(&"manifest.xml".to_string()), "{name}");
    }
}

/// Recompressing deflates every member and keeps the content.
#[test]
fn recompress_writes_every_member_deflated() {
    for name in ARCHIVES {
        let mut archive = Archive::from_bytes(&fixture(&format!("omex/{name}"))).unwrap();
        let written = archive.to_bytes_recompressed(9).unwrap();
        let reader = Reader::new(written.clone()).unwrap();
        assert!(!reader.members().is_empty(), "{name}");
        for member in reader.members() {
            assert_eq!(member.method, DEFLATE, "{name}: {:?}", member.name);
            assert_eq!(member.flags & ENCRYPTED, 0, "{name}: {:?}", member.name);
        }
        assert_same_entries(&mut archive, &written, name);
    }
}

/// An archive which was never read holds its manifest and nothing else.
#[test]
fn an_empty_archive_writes_a_valid_zip_with_one_member() {
    let written = Archive::new().to_bytes().unwrap();
    let reader = Reader::new(written.clone()).unwrap();
    let names: Vec<&str> = reader
        .members()
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, ["manifest.xml"]);
    let archive = Archive::from_bytes(&written).unwrap();
    assert_eq!(archive.manifest(), Archive::new().manifest());
}

/// An entry the manifest lists and the zip does not hold stays a manifest entry.
///
/// `iCGB21FR.omex` is a real archive from BioModels whose manifest lists
/// `./iCGB21FR_memote.html`, while the member is named `iCGB21_memote.html`.
#[test]
fn an_entry_the_archive_does_not_hold_is_only_a_manifest_entry() {
    let mut archive = Archive::from_bytes(&fixture("omex/iCGB21FR.omex")).unwrap();
    let stale = "./iCGB21FR_memote.html";
    assert!(archive.entry(stale).is_some());
    assert!(matches!(
        archive.entry_bytes(stale),
        Err(OmexError::NotFound(_))
    ));
    let written = archive.to_bytes().unwrap();
    let reader = Reader::new(written.clone()).unwrap();
    assert!(
        reader
            .members()
            .iter()
            .all(|member| member.name != member_name(stale))
    );
    let mut read_back = Archive::from_bytes(&written).unwrap();
    assert_eq!(read_back.entry(stale), archive.entry(stale));
    assert!(matches!(
        read_back.entry_bytes(stale),
        Err(OmexError::NotFound(_))
    ));
}

/// An encrypted member is written as plaintext, so the archive needs no password to be read.
///
/// The core does not write an encrypted archive: an encrypted member is decrypted and deflated
/// instead of being copied as the ciphertext it is, which would leave half an archive encrypted
/// and the regenerated manifest beside it in the clear.
#[test]
fn an_encrypted_member_is_written_as_plaintext() {
    let bytes = fixture("zip/encrypted_omex.zip");
    let mut archive = Archive::from_bytes_with_password(&bytes, b"test1234").unwrap();
    let written = archive.to_bytes().unwrap();
    let reader = Reader::new(written.clone()).unwrap();
    for member in reader.members() {
        assert_eq!(member.flags & ENCRYPTED, 0, "{:?}", member.name);
        assert_eq!(member.method, DEFLATE, "{:?}", member.name);
    }
    let mut read_back = Archive::from_bytes(&written).unwrap();
    assert_eq!(read_back.entry_bytes("./extra.txt").unwrap(), b"extra");
    assert_same_entries(&mut archive, &written, "encrypted_omex.zip");
}

/// A member of a method the core cannot inflate is copied as the blob it is all the same.
///
/// The copy leaves such a member exactly as readable as it was, which is better than refusing
/// to write the archive at all. Deflating it again is what cannot be done, and says so.
#[test]
fn a_member_of_an_unsupported_method_is_copied() {
    const BZIP2: u16 = 12;
    let bytes = hand_built_zip(&[
        Raw::new("manifest.xml", MANIFEST),
        Raw {
            method: BZIP2,
            ..Raw::new("secret.txt", b"not a bzip2 stream")
        },
    ]);
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    let reader = Reader::new(archive.to_bytes().unwrap()).unwrap();
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == "secret.txt")
        .unwrap();
    assert_eq!(member.method, BZIP2);
    assert_eq!(reader.raw(member).unwrap(), b"not a bzip2 stream");
    assert!(matches!(
        archive.to_bytes_recompressed(6),
        Err(OmexError::Unsupported(_))
    ));
}

/// An encrypted member without a password cannot be written, and is not copied as ciphertext.
#[test]
fn an_encrypted_member_without_a_password_is_refused() {
    let mut archive = Archive::from_bytes(&encrypted_member_archive()).unwrap();
    // the missing password is its own kind of refusal, and the message keeps the prefix of one
    assert!(matches!(archive.to_bytes(), Err(OmexError::Password(_))));
    assert!(matches!(
        archive.to_bytes_recompressed(6),
        Err(OmexError::Password(_))
    ));
    assert!(
        archive
            .to_bytes()
            .unwrap_err()
            .to_string()
            .starts_with("refused: ")
    );
}

/// The writer stores and deflates a member, and the reader reads both back.
#[test]
fn the_writer_adds_a_stored_and_a_deflated_member() {
    let data = b"the content of a member, repeated to give deflate something to do. ".repeat(20);
    let mut writer = Writer::new(Vec::new());
    writer.add_stored("stored.txt", &data).unwrap();
    writer.add_deflated("deflated.txt", &data, 6).unwrap();
    writer.add_stored("empty.bin", b"").unwrap();
    writer.add_deflated("empty.txt", b"", 6).unwrap();
    let reader = Reader::new(writer.into_bytes().unwrap()).unwrap();
    let names: Vec<&str> = reader
        .members()
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["stored.txt", "deflated.txt", "empty.bin", "empty.txt"]
    );
    for member in reader.members() {
        let expected: &[u8] = if member.name.starts_with("empty") {
            b""
        } else {
            &data
        };
        assert_eq!(
            reader.inflate(member).unwrap(),
            expected,
            "{:?}",
            member.name
        );
    }
    assert_eq!(reader.members()[0].method, STORED);
    assert_eq!(reader.members()[1].method, DEFLATE);
    assert!(reader.members()[1].compressed_size < reader.members()[0].compressed_size);
}

/// The writer refuses an encrypted member, whose flags a raw copy would drop.
#[test]
fn add_raw_refuses_an_encrypted_member() {
    let bytes = fixture("zip/encrypted_omex.zip");
    let reader = Reader::new(bytes).unwrap();
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == "extra.txt")
        .unwrap();
    let raw = reader.raw(member).unwrap();
    let mut writer = Writer::new(Vec::new());
    assert!(matches!(
        writer.add_raw("extra.txt", member, raw),
        Err(OmexError::Unsupported(_))
    ));
}

/// The writer refuses raw bytes which are not as many as the member declares.
#[test]
fn add_raw_refuses_bytes_of_another_length() {
    let bytes = fixture("omex/CompModels.omex");
    let reader = Reader::new(bytes).unwrap();
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == "README.md")
        .unwrap();
    let raw = reader.raw(member).unwrap();
    let mut writer = Writer::new(Vec::new());
    assert!(matches!(
        writer.add_raw("README.md", member, &raw[1..]),
        Err(OmexError::Refused(_))
    ));
}

/// The writer refuses a stored member whose two sizes differ, which no stored member has.
///
/// Such a central directory is malformed, and writing the member as it is declared would put
/// two different sizes into a local file header which says the member is stored.
#[test]
fn add_raw_refuses_a_stored_member_of_two_sizes() {
    let member = Member {
        name: "stored.bin".to_string(),
        method: STORED,
        crc32: crc32fast::hash(b"abc"),
        compressed_size: 3,
        uncompressed_size: 11,
        header_offset: 0,
        flags: 0,
        dos_time: STAMP.0,
        dos_date: STAMP.1,
        external_attributes: REGULAR_FILE,
    };
    let mut writer = Writer::new(Vec::new());
    let error = writer.add_raw("stored.bin", &member, b"abc").unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, OmexError::Malformed(_)), "{message}");
    assert!(message.contains('3') && message.contains("11"), "{message}");
    // the member of the same bytes which declares them as what it is is written
    let member = Member {
        uncompressed_size: 3,
        ..member
    };
    writer.add_raw("stored.bin", &member, b"abc").unwrap();
    assert_eq!(
        Reader::new(writer.into_bytes().unwrap())
            .unwrap()
            .members()
            .len(),
        1
    );
}

/// A member under WinZip AES is not written at all: no plaintext for it, and no copy either.
///
/// The fixture `zip/aes.zip` is where the member comes from. It is no COMBINE archive, so the
/// archive which runs the write is built around that member, with the method, the flags and the
/// bytes it has.
#[test]
fn an_aes_member_is_not_written() {
    const AES: u16 = 99;
    let input = Reader::new(fixture("zip/aes.zip")).unwrap();
    let member = &input.members()[0];
    assert_eq!(
        (member.method, member.flags & ENCRYPTED),
        (AES, ENCRYPTED),
        "the fixture"
    );
    let raw = input.raw(member).unwrap();
    // a copy would carry the encryption flag of the member into an archive of plain members
    let mut writer = Writer::new(Vec::new());
    assert!(matches!(
        writer.add_raw(&member.name, member, raw),
        Err(OmexError::Unsupported(_))
    ));
    let bytes = hand_built_zip(&[
        Raw::new("manifest.xml", MANIFEST),
        Raw {
            method: AES,
            flags: ENCRYPTED,
            ..Raw::new("secret.txt", raw)
        },
    ]);
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    // and the core does not decrypt AES, so there is nothing to write as the plaintext either
    assert!(matches!(archive.to_bytes(), Err(OmexError::Unsupported(_))));
    assert!(matches!(
        archive.to_bytes_recompressed(6),
        Err(OmexError::Unsupported(_))
    ));
}

/// A name longer than the field which holds it is refused and not truncated.
#[test]
fn refuses_a_name_longer_than_the_field_which_holds_it() {
    let mut writer = Writer::new(Vec::new());
    let name = "a".repeat(usize::from(u16::MAX) + 1);
    assert!(matches!(
        writer.add_stored(&name, b"x"),
        Err(OmexError::Refused(_))
    ));
    let name = "a".repeat(usize::from(u16::MAX));
    writer.add_stored(&name, b"x").unwrap();
    assert_eq!(
        Reader::new(writer.into_bytes().unwrap())
            .unwrap()
            .members()
            .len(),
        1
    );
}

/// A compression level no deflate knows is refused, and never reaches the compressor.
#[test]
fn refuses_a_compression_level_no_deflate_knows() {
    let mut writer = Writer::new(Vec::new());
    assert!(matches!(
        writer.add_deflated("a.txt", b"x", 10),
        Err(OmexError::Refused(_))
    ));
    let mut archive = Archive::new();
    assert!(matches!(
        archive.to_bytes_recompressed(10),
        Err(OmexError::Refused(_))
    ));
    // the whole range deflate knows is written and read back
    for level in 0..=9 {
        let mut writer = Writer::new(Vec::new());
        writer.add_deflated("a.txt", b"content", level).unwrap();
        let reader = Reader::new(writer.into_bytes().unwrap()).unwrap();
        assert_eq!(reader.inflate(&reader.members()[0]).unwrap(), b"content");
    }
}

/// More members than the 16 bit count field holds are written with the zip64 records.
///
/// The sizes and the offsets of a zip64 archive cannot be reached in a test, which would have
/// to write more than 4 GB; the member count can, and it exercises the zip64
/// end-of-central-directory record, its locator and the sentinels of the 32 bit record.
#[test]
fn writes_the_zip64_end_record_above_the_member_count_field() {
    let below = write_members(u32::from(u16::MAX) - 1);
    assert!(!holds_zip64_record(&below));
    assert_eq!(
        Reader::new(below).unwrap().members().len(),
        usize::from(u16::MAX) - 1
    );
    let above = write_members(u32::from(u16::MAX));
    assert!(holds_zip64_record(&above));
    let reader = Reader::new(above).unwrap();
    assert_eq!(reader.members().len(), usize::from(u16::MAX));
    let names: BTreeSet<&str> = reader
        .members()
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names.len(), usize::from(u16::MAX));
}

/// An archive of `count` stored members, named after their index.
fn write_members(count: u32) -> Vec<u8> {
    let mut writer = Writer::new(Vec::new());
    for index in 0..count {
        writer.add_stored(&format!("{index}.bin"), b"").unwrap();
    }
    writer.into_bytes().unwrap()
}

/// Does the archive hold a zip64 end-of-central-directory record?
fn holds_zip64_record(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| window == b"PK\x06\x06")
}

/// An archive of a plaintext manifest and one member which claims to be encrypted.
///
/// `tests/data/zip/encrypted_omex.zip` has an encrypted manifest, so it cannot be opened
/// without its password at all; this one opens and fails when the member is reached. The
/// ciphertext is 12 bytes of encryption header and 5 bytes no password ever decrypts, which the
/// write never reaches: it is refused for the missing password first.
fn encrypted_member_archive() -> Vec<u8> {
    hand_built_zip(&[
        Raw::new("manifest.xml", MANIFEST),
        Raw {
            flags: ENCRYPTED,
            ..Raw::new("secret.txt", &[0xa5; 17])
        },
    ])
}

/// The manifest of the hand built archives, which lists one entry besides the required ones.
const MANIFEST: &[u8] = br#"<?xml version="1.0"?>
<omexManifest xmlns="http://identifiers.org/combine.specifications/omex-manifest">
  <content location="." format="http://identifiers.org/combine.specifications/omex"/>
  <content location="./manifest.xml" format="http://identifiers.org/combine.specifications/omex-manifest"/>
  <content location="./secret.txt" format="http://purl.org/NET/mediatypes/text/plain"/>
</omexManifest>"#;

/// One member of a hand built archive.
struct Raw<'a> {
    /// The name of the member.
    name: &'a str,
    /// Its bytes, as the archive stores them.
    data: &'a [u8],
    /// Its compression method, which the archive only declares.
    method: u16,
    /// Its general purpose bit flags.
    flags: u16,
}

impl<'a> Raw<'a> {
    /// A stored member of no flags, stamped with [`STAMP`] as the fixtures are.
    fn new(name: &'a str, data: &'a [u8]) -> Self {
        Self {
            name,
            data,
            method: STORED,
            flags: 0,
        }
    }
}

/// Build a zip archive of `members`, each as the bytes, the method and the flags it is given.
///
/// The writer is under test here, so the input of a test is assembled by hand and not written
/// with it. The data is stored as it is handed over, whatever the method declares, which is
/// what a test of a member the core cannot inflate needs. Every member is stamped with
/// [`STAMP`], so that a copy of it is told from a member the writer compressed itself.
fn hand_built_zip(members: &[Raw]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for member in members {
        let (name, data) = (member.name, member.data);
        let offset = out.len() as u32;
        let size = data.len() as u32;
        let mut common = vec![20, 0];
        common.extend_from_slice(&member.flags.to_le_bytes());
        common.extend_from_slice(&member.method.to_le_bytes());
        common.extend_from_slice(&STAMP.0.to_le_bytes());
        common.extend_from_slice(&STAMP.1.to_le_bytes());
        common.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes());
        common.extend_from_slice(&size.to_le_bytes());
        common.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&common);
        out.extend_from_slice(&[0, 0]); // no extra field
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
    out.extend_from_slice(&[0, 0]); // no archive comment
    out
}

/// Write the archives of the corpus to `target/tmp`, where `scripts/read_back.py` reads them.
///
/// The rust suite must not depend on a python environment, so what python's `zipfile` and
/// `pymetadata` make of these archives is checked by that script, by hand. The test itself
/// asserts what it can without python: the written archive reads back through the core.
#[test]
fn writes_the_corpus_for_read_back_py() {
    let directory = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("write");
    std::fs::create_dir_all(&directory).unwrap();
    for name in ARCHIVES {
        let mut archive = Archive::from_bytes(&fixture(&format!("omex/{name}"))).unwrap();
        let written = archive.to_bytes().unwrap();
        std::fs::write(directory.join(name), &written).unwrap();
        let recompressed = archive.to_bytes_recompressed(9).unwrap();
        std::fs::write(
            directory.join(format!("recompressed_{name}")),
            &recompressed,
        )
        .unwrap();
        assert_same_entries(&mut archive, &written, name);
    }
}

/// The 32 bit record of a zip64 archive carries the fields which fit, as `zipfile` writes them.
#[test]
fn the_32_bit_record_keeps_what_fits_under_zip64() {
    let bytes = write_members(u32::from(u16::MAX));
    assert!(holds_zip64_record(&bytes));
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    // the two counts carry the sentinel, which is what 65535 members are in a 16 bit field
    assert_eq!(&bytes[end + 8..end + 12], [0xff; 4]);
    // the central directory of this archive is 3 MB at an offset of 3 MB, and both fit
    let cd_size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
    let cd_offset = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap());
    assert!(cd_size > 0 && cd_size < u32::MAX, "{cd_size}");
    assert!(cd_offset > 0 && cd_offset < u32::MAX, "{cd_offset}");
    assert_eq!(u64::from(cd_offset) + u64::from(cd_size), (end - 76) as u64);
}

/// A streamed archive is the archive `to_bytes` returns, byte for byte.
#[test]
fn the_stream_writes_what_to_bytes_writes() {
    for name in ARCHIVES {
        let bytes = fixture(&format!("omex/{name}"));
        let mut archive = Archive::from_bytes(&bytes).unwrap();
        let in_memory = archive.to_bytes().unwrap();

        let mut archive = Archive::from_bytes(&bytes).unwrap();
        let mut streamed = Vec::new();
        let written = archive.write_into(&mut streamed).unwrap();

        assert_eq!(streamed, in_memory, "{name}");
        assert_eq!(written, streamed.len() as u64, "{name}: the count of bytes");
    }
}

/// The same holds for a write which deflates everything again, and for an edited archive.
#[test]
fn the_stream_writes_what_to_bytes_writes_for_a_changed_archive() {
    let bytes = fixture("omex/CompModels.omex");
    let edit = |archive: &mut Archive| {
        archive
            .set_entry(
                Entry::new("./added.txt", "text/plain", false).unwrap(),
                b"content".to_vec(),
            )
            .unwrap();
        archive.remove_entry("./manifest.xml").ok();
    };

    let mut first = Archive::from_bytes(&bytes).unwrap();
    edit(&mut first);
    let in_memory = first.to_bytes_recompressed(6).unwrap();
    let mut second = Archive::from_bytes(&bytes).unwrap();
    edit(&mut second);
    let mut streamed = Vec::new();
    second.write_into_recompressed(&mut streamed, 6).unwrap();

    assert_eq!(streamed, in_memory);
}

/// An archive built from scratch streams as well, and a reader opens what was streamed.
#[test]
fn a_streamed_archive_is_read_back() {
    let mut archive = Archive::new();
    archive
        .set_entry(
            Entry::new("./model.xml", "application/xml", true).unwrap(),
            b"<sbml/>".to_vec(),
        )
        .unwrap();
    let mut streamed = Vec::new();
    archive.write_into(&mut streamed).unwrap();

    let mut read = Archive::from_bytes(&streamed).unwrap();
    assert_eq!(read.entry_bytes("./model.xml").unwrap(), b"<sbml/>");
}

/// A sink which refuses the bytes is an `Io` error, which no other failure is.
#[test]
fn a_sink_which_fails_is_an_io_error() {
    /// A sink which takes `budget` bytes and fails afterwards.
    struct Tight {
        budget: usize,
    }

    impl std::io::Write for Tight {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf.len() > self.budget {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "no space left on device",
                ));
            }
            self.budget -= buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let bytes = fixture("omex/CompModels.omex");
    let mut archive = Archive::from_bytes(&bytes).unwrap();
    let error = archive.write_into(Tight { budget: 64 }).unwrap_err();

    assert!(matches!(error, OmexError::Io(_)), "{error:?}");
    assert!(error.to_string().starts_with("io: "), "{error}");
    assert!(
        error.to_string().contains("no space left"),
        "the error of the sink is kept: {error}"
    );
    // the archive is unchanged: the same write into a sink which takes the bytes works
    let mut good = Vec::new();
    archive.write_into(&mut good).unwrap();
    assert_eq!(good, archive.to_bytes().unwrap());
}
