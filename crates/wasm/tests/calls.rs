//! The calls behind the exports, exercised natively.
//!
//! Every export of this host is a wrapper which moves bytes and nothing else, so what is tested
//! here is the whole of the host but the wrappers: the pure functions of
//! `pymetadata_wasm::calls`, which take and return owned data and touch no module state. The
//! wrappers and the module state they reach are tested by the unit tests of
//! `crates/wasm/src/lib.rs`, and the module they are built into by
//! `crates/wasm/tests/imports.rs`.

use pymetadata_core::wire::ManifestResponse;
use pymetadata_core::{Archive, lookup_format};
use pymetadata_wasm::calls;
use serde_json::{Value, json};

/// An archive of the corpus: six entries, three SBML models, a README and no unlisted member.
const COMP_MODELS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../core/tests/data/omex/CompModels.omex"
));

/// The entries of `CompModels.omex`, in the order its manifest lists them.
const LOCATIONS: [&str; 6] = [
    ".",
    "./manifest.xml",
    "./README.md",
    "./models/omex_comp.xml",
    "./models/omex_comp_flat.xml",
    "./models/omex_minimal.xml",
];

/// The first bytes of `./README.md` of that archive, and how long it is.
const README: &[u8] = b"# Composite model COMBINE archive (OMEX)\n";
/// The length of `./README.md` of that archive, as python's `zipfile` reads it.
const README_LEN: usize = 103;

/// The request of `omex_set_entry`: the JSON header and the content bytes behind it.
fn set_request(location: &str, format: Option<&str>, master: bool, data: &[u8]) -> Vec<u8> {
    let mut header = json!({"location": location, "master": master, "len": data.len()});
    if let Some(format) = format {
        header["format"] = json!(format);
    }
    let mut request = serde_json::to_vec(&header).expect("the header");
    request.extend_from_slice(data);
    request
}

/// The JSON of an answer.
fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|error| {
        panic!(
            "the answer is no JSON ({error}): {}",
            String::from_utf8_lossy(bytes)
        )
    })
}

/// The entries of the manifest answer are the ones the archive lists, in order.
#[test]
fn open_then_manifest_returns_the_entries_as_json() {
    let archive = calls::open(COMP_MODELS).expect("the archive");
    let answer = calls::manifest(&archive).expect("the manifest");
    let json = parse(&answer);
    let entries = json["entries"].as_array().expect("the entries");
    let locations: Vec<&str> = entries
        .iter()
        .map(|entry| entry["location"].as_str().expect("a location"))
        .collect();
    assert_eq!(locations, LOCATIONS);
    assert_eq!(
        entries[3]["format"],
        json!("http://identifiers.org/combine.specifications/sbml.level-3.version-1")
    );
    assert_eq!(entries[3]["master"], json!(false));
    // and the answer is the JSON of the wire type, which is the contract the loader reads
    let expected = ManifestResponse {
        entries: archive.manifest().entries().to_vec(),
    };
    assert_eq!(answer, serde_json::to_vec(&expected).expect("the response"));
}

/// The bytes of an entry are the inflated ones, and the manifest is refused.
#[test]
fn entry_returns_the_inflated_bytes() {
    let mut archive = calls::open(COMP_MODELS).expect("the archive");
    let bytes = calls::entry(&mut archive, b"./README.md").expect("the entry");
    assert_eq!(bytes.len(), README_LEN);
    assert!(
        bytes.starts_with(README),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    // a spelling of the same location, which the core normalizes
    assert_eq!(
        calls::entry(&mut archive, b"README.md").expect("again"),
        bytes
    );
    // the bytes of the manifest are written from the entries, so asking for them is refused
    assert_eq!(
        calls::entry(&mut archive, b"./manifest.xml")
            .expect_err("refused")
            .kind(),
        "refused"
    );
    // an entry which is not there, and one whose location is no UTF-8
    assert_eq!(
        calls::entry(&mut archive, b"./nothing.txt")
            .expect_err("not found")
            .kind(),
        "not_found"
    );
    assert_eq!(
        calls::entry(&mut archive, &[0xff, 0xfe])
            .expect_err("no UTF-8")
            .kind(),
        "malformed"
    );
}

/// An entry set and written is in the archive which is read back.
#[test]
fn set_entry_then_write_produces_a_readable_archive() {
    let mut archive = calls::open(COMP_MODELS).expect("the archive");
    let data = b"time,value\n0,1\n1,2\n";
    // no format in the request, so the host guesses it from the suffix
    let request = set_request("./data/timecourse.csv", None, true, data);
    assert!(
        calls::set_entry(&mut archive, &request)
            .expect("set")
            .is_empty()
    );
    let written = calls::write(&mut archive, false).expect("the archive bytes");

    let mut reopened = Archive::from_bytes(&written).expect("a readable archive");
    let locations: Vec<&str> = reopened.locations().collect();
    assert_eq!(locations.len(), LOCATIONS.len() + 1);
    assert!(
        locations.contains(&"./data/timecourse.csv"),
        "{locations:?}"
    );
    let entry = reopened.entry("./data/timecourse.csv").expect("the entry");
    assert_eq!(entry.format, lookup_format("csv"));
    assert!(entry.master);
    assert_eq!(
        reopened
            .entry_bytes("./data/timecourse.csv")
            .expect("bytes"),
        data
    );
    // and the entries which were never replaced came through the copy unchanged
    assert_eq!(
        reopened.entry_bytes("./README.md").expect("bytes").len(),
        README_LEN
    );

    // the entry is gone from the archive which is written after it was removed
    assert!(
        calls::remove_entry(&mut archive, b"./data/timecourse.csv")
            .expect("removed")
            .is_empty()
    );
    let written = calls::write(&mut archive, true).expect("the recompressed bytes");
    let reopened = Archive::from_bytes(&written).expect("a readable archive");
    assert_eq!(reopened.locations().collect::<Vec<&str>>(), LOCATIONS);
    // removing it twice says so, rather than reporting a second removal
    assert_eq!(
        calls::remove_entry(&mut archive, b"./data/timecourse.csv")
            .expect_err("not found")
            .kind(),
        "not_found"
    );
    // and the two entries the specification requires are not removed, where a location which is
    // no location of an archive is the refusal it is for every other call
    for location in [&b"."[..], b"./manifest.xml"] {
        assert_eq!(
            calls::remove_entry(&mut archive, location)
                .expect_err("not found")
                .kind(),
            "not_found",
            "{}",
            String::from_utf8_lossy(location)
        );
    }
    assert_eq!(
        calls::remove_entry(&mut archive, b"../escape.txt")
            .expect_err("refused")
            .kind(),
        "refused"
    );
    assert_eq!(
        calls::remove_entry(&mut archive, &[0xff])
            .expect_err("no UTF-8")
            .kind(),
        "malformed"
    );
}

/// Bytes which are no archive are reported as malformed, with a message.
#[test]
fn a_malformed_archive_answers_with_the_malformed_kind() {
    let error = calls::open(b"PK\x03\x04 and nothing else").expect_err("malformed");
    assert_eq!(error.kind(), "malformed");
    let answer = parse(&error.response());
    // the shape of `wire::ErrorResponse`: the kind and the message, and nothing else
    assert_eq!(answer["kind"], json!("malformed"));
    let message = answer["message"].as_str().expect("a message");
    assert!(message.contains("malformed"), "{answer}");
    assert_eq!(answer.as_object().expect("an object").len(), 2, "{answer}");
    // and an empty input is no archive either
    assert_eq!(calls::open(&[]).expect_err("malformed").kind(), "malformed");
}

/// A request which is not the header and the bytes it declares leaves the archive alone.
#[test]
fn a_set_entry_request_is_checked_before_the_archive_is_touched() {
    let mut archive = calls::open(COMP_MODELS).expect("the archive");
    let before = archive.locations().count();
    for request in [
        // two bytes declared, one sent
        &br#"{"location":"./a.txt","len":2}a"#[..],
        // one byte declared, two sent
        &br#"{"location":"./a.txt","len":1}ab"#[..],
        // no JSON at all
        &b"not a header"[..],
        // a truncated header
        &br#"{"location":"./a.txt","#[..],
        // an unknown key, which `SetEntryRequest` refuses
        &br#"{"location":"./a.txt","len":0,"mastr":true}"#[..],
        // no length
        &br#"{"location":"./a.txt"}"#[..],
    ] {
        let error = calls::set_entry(&mut archive, request).expect_err("malformed");
        assert_eq!(
            error.kind(),
            "malformed",
            "{}",
            String::from_utf8_lossy(request)
        );
    }
    // a location outside of the archive is refused, and the manifest is not set from bytes
    for location in ["../escape.txt", "./manifest.xml", "."] {
        let request = set_request(location, None, false, b"x");
        assert_eq!(
            calls::set_entry(&mut archive, &request)
                .expect_err("refused")
                .kind(),
            "refused",
            "{location}"
        );
    }
    assert_eq!(archive.locations().count(), before, "nothing was added");
}

/// What this build is.
#[test]
fn about_names_the_version_and_the_backend() {
    let answer = parse(&calls::about());
    assert_eq!(answer["version"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(answer["backend"], json!("zlib-rs"));
}
