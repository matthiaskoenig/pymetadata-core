//! Traditional PKWARE encryption, "ZipCrypto", the one scheme python's `zipfile` decrypts.
//!
//! The cipher is weak and is not here for its security: a COMBINE archive which a user
//! protected with `zip -e` has to read the way the reference implementation reads it, and that
//! is all this module does. WinZip AES and strong encryption are refused by the reader before
//! the bytes get here.

use super::spec::DATA_DESCRIPTOR;
use super::{Member, OmexError};

/// The length of the encryption header in front of the data of a member.
const HEADER_LEN: usize = 12;
/// The initial values of the three keys.
const INITIAL_KEYS: [u32; 3] = [0x1234_5678, 0x2345_6789, 0x3456_7890];
/// The multiplier of the second key.
const MULTIPLIER: u32 = 134_775_813;

/// The table of the CRC32 with the polynomial `0xedb88320`, which the key update steps through.
const CRC_TABLE: [u32; 256] = crc_table();

/// Build [`CRC_TABLE`].
const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0xedb8_8320
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// Step a CRC32 register by one byte.
fn crc_step(crc: u32, byte: u8) -> u32 {
    // the index is one byte and the table has 256 entries, so `get` never misses; it is `get`
    // all the same, so that nothing which depends on host bytes is indexed. The one index in
    // this module, in `crc_table`, runs at compile time over a loop bound by the table length,
    // where an out of bounds index would fail the build and not an archive
    let entry = CRC_TABLE
        .get(usize::from((crc as u8) ^ byte))
        .copied()
        .unwrap_or_default();
    entry ^ (crc >> 8)
}

/// The three keys of the cipher.
struct Keys([u32; 3]);

impl Keys {
    /// The keys after `password` was mixed in.
    fn new(password: &[u8]) -> Self {
        let mut keys = Self(INITIAL_KEYS);
        for byte in password {
            keys.update(*byte);
        }
        keys
    }

    /// Mix one plaintext byte into the keys.
    fn update(&mut self, plain: u8) {
        let [first, second, third] = self.0;
        let first = crc_step(first, plain);
        let second = second
            .wrapping_add(first & 0xff)
            .wrapping_mul(MULTIPLIER)
            .wrapping_add(1);
        let third = crc_step(third, (second >> 24) as u8);
        self.0 = [first, second, third];
    }

    /// Decrypt one byte, and mix the plaintext into the keys.
    fn decrypt(&mut self, cipher: u8) -> u8 {
        let [_, _, third] = self.0;
        let temp = (third | 2) & 0xffff;
        let plain = cipher ^ ((temp.wrapping_mul(temp ^ 1) >> 8) as u8);
        self.update(plain);
        plain
    }
}

/// Decrypt the data of `member`, which is `raw` as the archive stores it.
///
/// The first 12 bytes are an encryption header of random bytes whose last one is a check on the
/// password: the high byte of the CRC32 of the member, or the high byte of its modification
/// time when bit 3 of the flags says the CRC32 was not known when the header was written. That
/// is the rule python's `zipfile` follows. One byte passes a wrong password 1 time in 256, and
/// the CRC32 check of the reader catches those.
///
/// # Errors
///
/// [`OmexError::Password`] if the password does not match the header,
/// [`OmexError::Malformed`] if the member is shorter than its encryption header.
pub(super) fn decrypt(member: &Member, raw: &[u8], password: &[u8]) -> Result<Vec<u8>, OmexError> {
    let Some((header, data)) = raw.split_at_checked(HEADER_LEN) else {
        return Err(OmexError::Malformed(format!(
            "member {:?} is encrypted and holds {} bytes, fewer than its encryption header of \
             {HEADER_LEN}",
            member.name,
            raw.len()
        )));
    };
    let check = if member.flags & DATA_DESCRIPTOR == 0 {
        (member.crc32 >> 24) as u8
    } else {
        (member.dos_time >> 8) as u8
    };
    let mut keys = Keys::new(password);
    let last = header.iter().fold(0, |_, byte| keys.decrypt(*byte));
    if last != check {
        return Err(OmexError::Password(format!(
            "the password does not open member {:?}",
            member.name
        )));
    }
    Ok(data.iter().map(|byte| keys.decrypt(*byte)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the one of the CRC32, whose first entries and check value are known.
    #[test]
    fn builds_the_crc32_table() {
        assert_eq!(CRC_TABLE[..3], [0, 0x7707_3096, 0xee0e_612c]);
        let crc = b"123456789"
            .iter()
            .fold(!0, |crc, byte| crc_step(crc, *byte));
        assert_eq!(!crc, 0xcbf4_3926);
    }
}
