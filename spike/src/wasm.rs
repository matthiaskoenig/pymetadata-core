//! Spike wasm surface: a raw C ABI with no imports.

use core::ffi::c_int;

static mut INPUT: Vec<u8> = Vec::new();
static mut OUTPUT: Vec<u8> = Vec::new();

/// Reserve `len` bytes of input and return the pointer to write into.
#[unsafe(no_mangle)]
pub extern "C" fn omex_input_alloc(len: usize) -> *mut u8 {
    unsafe {
        let input = &mut *(&raw mut INPUT);
        input.clear();
        input.resize(len, 0);
        input.as_mut_ptr()
    }
}

/// Number of manifest entries of the archive in the input buffer, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn omex_manifest_count() -> c_int {
    let input = unsafe { &*(&raw const INPUT) };
    match crate::manifest_only(input) {
        Ok(entries) => entries.len() as c_int,
        Err(_) => -1,
    }
}

/// Replace the first entry with 64 kB and write the archive into the output
/// buffer. Returns 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn omex_modify_first() -> c_int {
    let input = unsafe { &*(&raw const INPUT) };
    let entries = match crate::manifest_only(input) {
        Ok(e) => e,
        Err(_) => return -1,
    };
    let location = match entries.iter().find(|e| e.location != "./manifest.xml") {
        Some(e) => e.location.trim_start_matches("./").to_string(),
        None => return -2,
    };
    let payload = vec![b'x'; 64 * 1024];
    match crate::modify_rawcopy(input, &location, &payload) {
        Ok(bytes) => {
            unsafe {
                let output = &mut *(&raw mut OUTPUT);
                *output = bytes;
            }
            0
        }
        Err(_) => -3,
    }
}

/// Pointer to the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn omex_output_ptr() -> *const u8 {
    unsafe { (*(&raw const OUTPUT)).as_ptr() }
}

/// Length of the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn omex_output_len() -> usize {
    unsafe { (*(&raw const OUTPUT)).len() }
}
