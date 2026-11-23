//! Zero-import check: flate2 + roxmltree + crc32fast, no zip crate.
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use std::io::{Read, Write};

static mut BUF: Vec<u8> = Vec::new();

/// Inflate raw deflate data, return inflated length.
#[unsafe(no_mangle)]
pub extern "C" fn inflate_len(ptr: *const u8, len: usize) -> i64 {
    let src = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut out = Vec::new();
    match DeflateDecoder::new(src).read_to_end(&mut out) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Deflate data into the static buffer, return its length.
#[unsafe(no_mangle)]
pub extern "C" fn deflate_len(ptr: *const u8, len: usize) -> i64 {
    let src = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    if enc.write_all(src).is_err() {
        return -1;
    }
    match enc.finish() {
        Ok(v) => {
            let n = v.len();
            unsafe { *(&mut *(&raw mut BUF)) = v };
            n as i64
        }
        Err(_) => -1,
    }
}

/// Count elements in an XML document given as UTF-8 bytes.
#[unsafe(no_mangle)]
pub extern "C" fn xml_count(ptr: *const u8, len: usize) -> i64 {
    let src = unsafe { std::slice::from_raw_parts(ptr, len) };
    let Ok(text) = std::str::from_utf8(src) else { return -1 };
    match roxmltree::Document::parse(text) {
        Ok(doc) => doc.descendants().filter(|n| n.is_element()).count() as i64,
        Err(_) => -1,
    }
}

/// CRC32 of a buffer, as every zip entry needs.
#[unsafe(no_mangle)]
pub extern "C" fn crc32(ptr: *const u8, len: usize) -> u32 {
    let src = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut h = crc32fast::Hasher::new();
    h.update(src);
    h.finalize()
}

static mut IN: Vec<u8> = Vec::new();

/// Reserve `len` input bytes and return the pointer to write into.
#[unsafe(no_mangle)]
pub extern "C" fn input_alloc(len: usize) -> *mut u8 {
    unsafe {
        let b = &mut *(&raw mut IN);
        b.clear();
        b.resize(len, 0);
        b.as_mut_ptr()
    }
}

/// Pointer to the static output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn out_ptr() -> *const u8 {
    unsafe { (*(&raw const BUF)).as_ptr() }
}
