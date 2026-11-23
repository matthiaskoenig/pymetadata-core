//! The zip container reader, against the archive corpus and the hand built edge cases.
//!
//! The corpus is the four COMBINE archives of `tests/data/omex/`; `scripts/members_oracle.py`
//! writes what python's `zipfile` reads from them to `tests/data/members.json`. The edge cases
//! of `tests/data/zip/` are built by `scripts/zip_edge_fixtures.py`, also with `zipfile`.

use std::collections::{BTreeMap, BTreeSet};

use pymetadata_core::OmexError;
use pymetadata_core::zip::{MAX_INFLATED_SIZE, Reader};
use sha2::{Digest, Sha256};

/// The archives of the corpus.
const ARCHIVES: [&str; 4] = [
    "BIOMD0000000001.omex",
    "CombineArchiveShowCase.omex",
    "CompModels.omex",
    "iCGB21FR.omex",
];

/// One member of an archive as python's `zipfile` reports it.
#[derive(Debug, serde::Deserialize)]
struct Oracle {
    /// The member name, as stored in the central directory.
    name: String,
    /// The compression method.
    method: u16,
    /// The CRC32 of the uncompressed bytes.
    crc32: u32,
    /// The uncompressed size.
    size: u64,
    /// The SHA256 of the uncompressed bytes, hex encoded.
    sha256: String,
}

/// Read a fixture of `crates/core/tests/data/`.
fn fixture(relative: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{relative}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {path}: {error}"))
}

/// The hex encoded SHA256 of `bytes`.
fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Every archive of the corpus lists its members, under unique names, `manifest.xml` among them.
#[test]
fn reads_the_members_of_every_fixture() {
    for archive in ARCHIVES {
        let bytes = fixture(&format!("omex/{archive}"));
        let reader = Reader::new(&bytes).unwrap_or_else(|error| panic!("{archive}: {error}"));
        let names: Vec<&str> = reader
            .members()
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        assert!(!names.is_empty(), "{archive} has no member");
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "{archive} repeats a member name");
        assert!(
            names.contains(&"manifest.xml"),
            "{archive} has no manifest.xml, only {names:?}"
        );
    }
}

/// Every member of the corpus inflates to the bytes python's `zipfile` reads, in the same order.
#[test]
fn inflate_matches_python_zipfile() {
    let text = include_str!("data/members.json");
    let expected: BTreeMap<String, Vec<Oracle>> = serde_json::from_str(text).unwrap();
    assert_eq!(expected.len(), ARCHIVES.len());
    for (archive, oracles) in &expected {
        let bytes = fixture(&format!("omex/{archive}"));
        let reader = Reader::new(&bytes).unwrap_or_else(|error| panic!("{archive}: {error}"));
        assert_eq!(
            reader.members().len(),
            oracles.len(),
            "{archive} member count"
        );
        for (member, oracle) in reader.members().iter().zip(oracles) {
            let at = format!("{archive}/{}", oracle.name);
            assert_eq!(member.name, oracle.name, "{at} name");
            assert_eq!(member.method, oracle.method, "{at} method");
            assert_eq!(member.crc32, oracle.crc32, "{at} crc32");
            assert_eq!(member.uncompressed_size, oracle.size, "{at} size");
            let data = reader
                .inflate(member)
                .unwrap_or_else(|error| panic!("{at}: {error}"));
            assert_eq!(data.len() as u64, oracle.size, "{at} inflated size");
            assert_eq!(sha256(&data), oracle.sha256, "{at} sha256");
            // a stored member is handed out raw byte for byte, a deflated one is shorter
            if member.method == 0 {
                assert_eq!(reader.raw(member).unwrap(), data.as_slice(), "{at} raw");
            } else {
                assert_eq!(
                    reader.raw(member).unwrap().len() as u64,
                    member.compressed_size,
                    "{at} raw size"
                );
            }
        }
    }
}

/// A stored member is handed out as the bytes it holds, raw and inflated alike.
#[test]
fn reads_a_stored_member() {
    let bytes = fixture("zip/stored.zip");
    let reader = Reader::new(&bytes).unwrap();
    assert_eq!(reader.members().len(), 1);
    let member = &reader.members()[0];
    assert_eq!(member.name, "stored.txt");
    assert_eq!(member.method, 0);
    assert_eq!(member.compressed_size, member.uncompressed_size);
    let text = b"a stored member of a hand built zip\n";
    assert_eq!(reader.raw(member).unwrap(), text);
    assert_eq!(reader.inflate(member).unwrap(), text);
}

/// The zip64 end-of-central-directory record and the zip64 extra field carry the real sizes,
/// the count and the local header offset.
#[test]
fn reads_zip64_sizes() {
    let bytes = fixture("zip/zip64.zip");
    let reader = Reader::new(&bytes).unwrap();
    // the end-of-central-directory record holds 0xffff entries, the zip64 record the true two
    assert_eq!(reader.members().len(), 2);
    let names: Vec<&str> = reader
        .members()
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, ["first.bin", "second.bin"]);
    let first = &reader.members()[0];
    let second = &reader.members()[1];
    // the 32 bit fields of both central directory records are 0xffffffff
    assert_eq!(first.uncompressed_size, 10_000);
    assert_eq!(first.compressed_size, 10_000);
    assert_eq!(first.header_offset, 0);
    assert_eq!(second.uncompressed_size, 20_000);
    assert_eq!(second.compressed_size, 20_000);
    assert!(second.header_offset > 10_000, "{}", second.header_offset);
    assert_eq!(reader.inflate(first).unwrap().len(), 10_000);
    assert_eq!(reader.inflate(second).unwrap().len(), 20_000);
}

/// A member written through an unseekable stream carries a data descriptor: the local header
/// holds zeros and the sizes of the central directory win.
#[test]
fn reads_a_member_with_a_data_descriptor() {
    for (archive, zeroed) in [
        ("zip/descriptors.zip", true),
        ("omex/CompModels.omex", false),
    ] {
        let bytes = fixture(archive);
        let reader = Reader::new(&bytes).unwrap();
        let members: Vec<_> = reader
            .members()
            .iter()
            .filter(|member| member.flags & 0x0008 != 0)
            .collect();
        assert!(!members.is_empty(), "{archive} has no data descriptor");
        for member in members {
            // the local header of a streamed member holds a zero CRC and a zero compressed size
            let local = usize::try_from(member.header_offset).unwrap();
            let zeros = &bytes[local + 14..local + 22];
            assert_eq!(zeros, [0; 8], "{archive}/{} local header", member.name);
            if zeroed {
                let size = &bytes[local + 22..local + 26];
                assert_eq!(size, [0; 4], "{archive}/{} local size", member.name);
            }
            assert!(member.compressed_size > 0, "{}", member.name);
            let data = reader.inflate(member).unwrap();
            assert_eq!(
                data.len() as u64,
                member.uncompressed_size,
                "{archive}/{}",
                member.name
            );
        }
    }
}

/// An archive behind a self extracting stub reads like the archive without it.
///
/// Every offset of its central directory is off by the length of whatever sits in front of the
/// archive, the offset of the zip64 record included, and the record at the end is what says by
/// how much. python's `zipfile` reads such an archive, so the core does too.
#[test]
fn reads_an_archive_behind_a_stub() {
    for (prepended, original) in [
        ("zip/prepended.zip", "omex/CompModels.omex"),
        ("zip/prepended_zip64.zip", "zip/zip64.zip"),
    ] {
        let behind = fixture(prepended);
        let plain = fixture(original);
        let stub = (behind.len() - plain.len()) as u64;
        assert_eq!(stub, 512, "{prepended}");
        let reader = Reader::new(&behind).unwrap_or_else(|error| panic!("{prepended}: {error}"));
        let source = Reader::new(&plain).unwrap();
        assert_eq!(
            reader.members().len(),
            source.members().len(),
            "{prepended}"
        );
        assert!(!reader.members().is_empty());
        for (member, expected) in reader.members().iter().zip(source.members()) {
            let at = format!("{prepended}/{}", member.name);
            assert_eq!(member.name, expected.name, "{at}");
            assert_eq!(member.crc32, expected.crc32, "{at}");
            assert_eq!(member.header_offset, expected.header_offset + stub, "{at}");
            assert_eq!(
                reader.inflate(member).unwrap(),
                source.inflate(expected).unwrap(),
                "{at}"
            );
        }
    }
}

/// A record which says the central directory starts behind where it ends is malformed.
///
/// Only data in front of the archive moves its offsets, and it moves them forward, so a
/// negative difference is a corrupt archive and not one to be corrected.
#[test]
fn refuses_a_central_directory_offset_behind_its_end() {
    let bytes = fixture("zip/bad_offset.zip");
    let error = Reader::new(&bytes).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
}

/// A truncated archive is malformed, however much of it is missing.
#[test]
fn refuses_a_truncated_archive() {
    // the archive without its end-of-central-directory record
    let bytes = fixture("zip/truncated.zip");
    assert!(matches!(Reader::new(&bytes), Err(OmexError::Malformed(_))));
    // and the same archive cut one byte, two bytes, 33 bytes and all but nothing short
    let full = fixture("zip/stored.zip");
    for cut in [0, 1, 32, full.len() - 1] {
        let short = &full[..full.len() - cut - 1];
        assert!(
            matches!(Reader::new(short), Err(OmexError::Malformed(_))),
            "cut {cut}"
        );
    }
}

/// A member whose bytes do not match the CRC of the central directory is malformed.
#[test]
fn refuses_a_bad_crc() {
    let bytes = fixture("zip/bad_crc.zip");
    let reader = Reader::new(&bytes).unwrap();
    let member = &reader.members()[0];
    let error = reader.inflate(member).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
    assert!(error.to_string().contains("CRC"), "{error}");
}

/// A member compressed with a method the core does not implement is unsupported, by name.
#[test]
fn refuses_an_unsupported_method() {
    let bytes = fixture("zip/bzip2.zip");
    let reader = Reader::new(&bytes).unwrap();
    let member = &reader.members()[0];
    assert_eq!(member.method, 12);
    let error = reader.inflate(member).unwrap_err();
    assert!(matches!(error, OmexError::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("bzip2"), "{error}");
    assert!(error.to_string().contains("12"), "{error}");
    // the member is still listed, and its stored bytes are still reachable
    assert_eq!(
        reader.raw(member).unwrap().len() as u64,
        member.compressed_size
    );
}

/// A central directory which declares more bytes than the archive holds is malformed, and
/// nothing is allocated for the declared size.
#[test]
fn refuses_an_oversized_member() {
    let bytes = fixture("zip/oversized.zip");
    let reader = Reader::new(&bytes).unwrap();
    // the first member claims to be stored over more bytes than the archive holds
    let compressed = &reader.members()[0];
    assert!(compressed.compressed_size > bytes.len() as u64);
    assert!(matches!(
        reader.raw(compressed),
        Err(OmexError::Malformed(_))
    ));
    assert!(matches!(
        reader.inflate(compressed),
        Err(OmexError::Malformed(_))
    ));
    // the second one claims that two deflated bytes inflate to 100 MB, which is beyond what
    // deflate can expand them to, and is refused before anything is allocated for it
    let uncompressed = &reader.members()[1];
    assert_eq!(uncompressed.compressed_size, 2);
    assert!(uncompressed.uncompressed_size > bytes.len() as u64);
    assert_eq!(reader.raw(uncompressed).unwrap().len(), 2);
    let error = reader.inflate(uncompressed).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
    assert!(error.to_string().contains("cannot hold"), "{error}");
}

/// A member which inflates to more than the reader allows is refused, and the host says how
/// much that is.
///
/// Every member of the corpus is orders of magnitude below the default, which is why
/// `inflate_matches_python_zipfile` reads all four archives under it.
#[test]
fn refuses_more_than_it_may_inflate() {
    // two compressed bytes which declare 2 GB are no member beyond the allowance, they are a
    // lie: raising the budget would not read them, so they are malformed and name both numbers
    let bytes = fixture("zip/oversized.zip");
    let reader = Reader::new(&bytes).unwrap();
    let member = &reader.members()[2];
    assert_eq!(member.name, "beyond_the_cap.txt");
    assert!(member.uncompressed_size > MAX_INFLATED_SIZE as u64);
    let error = reader.inflate(member).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
    let message = error.to_string();
    assert!(
        message.contains(&member.uncompressed_size.to_string()),
        "{message}"
    );
    assert!(
        message.contains(&format!("{} compressed bytes", member.compressed_size)),
        "{message}"
    );

    // a lowered allowance, against a member the default reads
    let plain = fixture("omex/CompModels.omex");
    let lowered = Reader::with_max_inflated_size(&plain, 100).unwrap();
    let default = Reader::new(&plain).unwrap();
    for (small, member) in lowered.members().iter().zip(default.members()) {
        if small.uncompressed_size > 100 {
            let error = lowered.inflate(small).unwrap_err();
            assert!(matches!(error, OmexError::Refused(_)), "{error}");
            assert!(error.to_string().contains(" 100 bytes"), "{error}");
            assert!(default.inflate(member).is_ok(), "{}", member.name);
        } else {
            assert!(lowered.inflate(small).is_ok(), "{}", small.name);
        }
    }

    // a declaration the compressed bytes can produce, which is merely above the allowance, is
    // refused before anything is allocated: 100 bytes of a deflate stream may hold 100 KB
    let plain = fixture("omex/CompModels.omex");
    let tight = Reader::with_max_inflated_size(&plain, 0).unwrap();
    let member = tight
        .members()
        .iter()
        .find(|m| m.method == 8 && m.uncompressed_size > 0)
        .unwrap();
    let error = tight.inflate(member).unwrap_err();
    assert!(matches!(error, OmexError::Refused(_)), "{error}");

    // the corpus leaves the default two orders of magnitude of room
    for archive in ARCHIVES {
        let bytes = fixture(&format!("omex/{archive}"));
        for member in Reader::new(&bytes).unwrap().members() {
            assert!(
                member.uncompressed_size < (MAX_INFLATED_SIZE / 100) as u64,
                "{archive}/{}",
                member.name
            );
        }
    }
}

/// A member whose stream inflates past the size the central directory declares is malformed.
#[test]
fn refuses_a_stream_longer_than_the_declared_size() {
    let bytes = fixture("zip/understated.zip");
    let reader = Reader::new(&bytes).unwrap();
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == "README.md")
        .unwrap();
    assert_eq!(member.method, 8);
    assert_eq!(member.uncompressed_size, 10);
    let error = reader.inflate(member).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
    // the message names the declared size and does not pin how far the stream went, which
    // depends on the allocator
    assert!(error.to_string().contains("says 10"), "{error}");
}

/// A deflate member of zero compressed bytes and zero uncompressed bytes is an empty member,
/// which is what .NET writes for an empty entry and what python's `zipfile` reads.
#[test]
fn reads_an_empty_deflate_member_of_no_bytes() {
    let bytes = fixture("zip/empty_deflate.zip");
    let reader = Reader::new(&bytes).unwrap();
    let member = &reader.members()[0];
    assert_eq!((member.method, member.compressed_size), (8, 0));
    assert_eq!(reader.inflate(member).unwrap(), b"");
}

/// A zero byte deflate stream which claims to hold bytes is malformed.
#[test]
fn refuses_a_zero_byte_stream_which_claims_bytes() {
    let mut bytes = fixture("zip/empty_deflate.zip");
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    bytes[central + 24..central + 28].copy_from_slice(&5u32.to_le_bytes());
    let reader = Reader::new(&bytes).unwrap();
    let error = reader.inflate(&reader.members()[0]).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
}
