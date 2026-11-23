//! ZipCrypto, the one encryption scheme the reference implementation reads.
//!
//! The fixtures of `tests/data/zip/` are built by `scripts/zip_edge_fixtures.py`, which
//! encrypts with its own implementation of the cipher and proves the result by reading it back
//! with python's `zipfile` and the password.

use pymetadata_core::OmexError;
use pymetadata_core::zip::Reader;

/// The encrypted fixtures: deflated, stored, and deflated under a data descriptor.
const ENCRYPTED: [&str; 3] = [
    "encrypted.zip",
    "encrypted_stored.zip",
    "encrypted_descriptor.zip",
];
/// The password of the encrypted fixtures.
const PASSWORD: &[u8] = b"test1234";
/// The content of the encrypted fixtures.
const SECRET: &[u8] = b"the secret content of an encrypted member\n\
the secret content of an encrypted member\n\
the secret content of an encrypted member\n";

/// Read a fixture of `crates/core/tests/data/zip/`.
fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/zip/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {path}: {error}"))
}

/// The one member of a fixture, read under `password`.
fn read(bytes: &[u8], password: Option<&[u8]>) -> Result<Vec<u8>, OmexError> {
    let reader = match password {
        Some(password) => Reader::new_with_password(bytes, password)?,
        None => Reader::new(bytes)?,
    };
    assert_eq!(reader.members().len(), 1);
    reader.inflate(&reader.members()[0])
}

/// The right password gives the plaintext, whether the header is checked against the CRC or,
/// under a data descriptor, against the modification time.
#[test]
fn reads_a_zipcrypto_member_with_the_right_password() {
    for name in ENCRYPTED {
        let bytes = fixture(name);
        assert_eq!(read(&bytes, Some(PASSWORD)).unwrap(), SECRET, "{name}");
    }
}

/// The fixture of a data descriptor is the one where the CRC of the local header is zero and
/// the check byte differs from the high byte of the CRC, so reading it proves the right byte.
#[test]
fn checks_the_header_against_the_time_under_a_data_descriptor() {
    let bytes = fixture("encrypted_descriptor.zip");
    let reader = Reader::new_with_password(&bytes, PASSWORD).unwrap();
    let member = &reader.members()[0];
    assert_eq!(member.flags & 0x09, 0x09);
    assert_ne!(member.crc32 >> 24, u32::from(member.dos_time >> 8));
    assert!(reader.inflate(member).is_ok());
}

/// A wrong password is refused, and the message says it is the password.
#[test]
fn refuses_a_wrong_password() {
    for name in ENCRYPTED {
        let bytes = fixture(name);
        for wrong in [&b"wrong"[..], b"", b"test12345", b"Test1234"] {
            let error = read(&bytes, Some(wrong)).unwrap_err();
            // a wrong password passes the one byte check 1 time in 256, which then fails the
            // CRC: none of these four does, and the message must not blame the archive
            assert!(matches!(error, OmexError::Refused(_)), "{name}: {error}");
            assert!(error.to_string().contains("password"), "{error}");
        }
    }
}

/// An encrypted member without a password is refused, and the message asks for one.
#[test]
fn refuses_an_encrypted_member_without_a_password() {
    for name in ENCRYPTED {
        let bytes = fixture(name);
        let error = read(&bytes, None).unwrap_err();
        assert!(matches!(error, OmexError::Refused(_)), "{name}: {error}");
        assert!(error.to_string().contains("password"), "{error}");
    }
}

/// An empty password is no password, as it is for `zipfile`.
#[test]
fn refuses_an_empty_password_like_no_password() {
    for name in ENCRYPTED {
        let bytes = fixture(name);
        let none = read(&bytes, None).unwrap_err();
        let empty = read(&bytes, Some(b"")).unwrap_err();
        assert!(matches!(empty, OmexError::Refused(_)), "{name}: {empty}");
        assert_eq!(none.to_string(), empty.to_string());
    }
}

/// The password and the allowance of a reader combine.
#[test]
fn combines_a_password_with_an_allowance() {
    let bytes = fixture("encrypted.zip");
    let mut reader = Reader::new_with_password(&bytes, PASSWORD).unwrap();
    reader.set_max_inflated_size(10);
    let error = reader.inflate(&reader.members()[0]).unwrap_err();
    assert!(matches!(error, OmexError::Refused(_)), "{error}");
    assert!(error.to_string().contains(" 10 bytes"), "{error}");
    reader.set_max_inflated_size(SECRET.len());
    assert_eq!(reader.inflate(&reader.members()[0]).unwrap(), SECRET);
}

/// A password given for an archive which is not encrypted is ignored, like `zipfile` does.
#[test]
fn ignores_a_password_for_a_plain_member() {
    let bytes = fixture("stored.zip");
    let with = read(&bytes, Some(PASSWORD)).unwrap();
    assert_eq!(with, read(&bytes, None).unwrap());
}

/// The raw bytes of an encrypted member are the stored ones, encryption header included.
#[test]
fn raw_keeps_the_encrypted_bytes() {
    let bytes = fixture("encrypted.zip");
    let reader = Reader::new(&bytes).unwrap();
    let member = &reader.members()[0];
    let raw = reader.raw(member).unwrap();
    assert_eq!(raw.len() as u64, member.compressed_size);
    assert_eq!(raw.len(), 12 + 5 + SECRET.len());
    assert!(!raw.windows(8).any(|window| SECRET.starts_with(window)));
}

/// WinZip AES is not read, with a password or without, and the message names it.
#[test]
fn refuses_aes() {
    let bytes = fixture("aes.zip");
    for password in [None, Some(PASSWORD)] {
        let error = read(&bytes, password).unwrap_err();
        assert!(matches!(error, OmexError::Unsupported(_)), "{error}");
        assert!(error.to_string().contains("AES"), "{error}");
    }
}

/// Strong encryption (bit 6) is not read either, which `zipfile` does not read.
#[test]
fn refuses_strong_encryption() {
    let mut bytes = fixture("encrypted.zip");
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    bytes[central + 8] |= 0x40;
    let error = read(&bytes, Some(PASSWORD)).unwrap_err();
    assert!(matches!(error, OmexError::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("strong"), "{error}");
}

/// An encrypted member which holds fewer bytes than the 12 of its header is malformed.
#[test]
fn refuses_a_member_shorter_than_its_header() {
    let mut bytes = fixture("encrypted.zip");
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    bytes[central + 20..central + 24].copy_from_slice(&11u32.to_le_bytes());
    let error = read(&bytes, Some(PASSWORD)).unwrap_err();
    assert!(matches!(error, OmexError::Malformed(_)), "{error}");
}
