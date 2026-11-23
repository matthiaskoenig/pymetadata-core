//! The WebAssembly host of `pymetadata-core`: a COMBINE archive in a browser.
//!
//! One `cdylib` for `wasm32-unknown-unknown` with a raw C ABI. There is no wasm-bindgen, no
//! filesystem, no reader or writer of the standard library and no import of any kind, which
//! `crates/wasm/tests/imports.rs` asserts: a page instantiates the module with an empty import
//! object, in a browser, a Web Worker or node, and a page which wants two cores runs one
//! instance per worker.
//!
//! # The protocol
//!
//! ```text
//! let ptr = omex_alloc(bytes.length);      // reserve the input buffer
//! new Uint8Array(memory.buffer).set(bytes, ptr);
//! if (omex_open() !== 0) { ... }           // the archive is the one in the input buffer
//! omex_manifest();                         // its entries, as JSON
//! const out = new Uint8Array(memory.buffer, omex_output_ptr(), omex_output_len());
//! ```
//!
//! Every export but the three of the buffers returns 0 for an answer and 1 for a failure, and
//! publishes exactly one output either way: the answer, or the JSON of
//! [`pymetadata_core::wire::ErrorResponse`], which names the kind of the failure. The six kinds
//! are the four of [`pymetadata_core::wire::kind`] - `malformed`, `refused`, `unsupported` and
//! `not_found` - and the two of this host, [`calls::INTERNAL`] and [`calls::PANIC`]. The code is
//! the bit a page branches on and the kind is what it routes on, which is the layout the sibling
//! hosts of `pkpdutils-core` and `sbmlsim-core` speak and the one the python binding reports by
//! the same strings.
//!
//! The exports:
//!
//! | export | what it does | its output |
//! |---|---|---|
//! | `omex_alloc(len) -> *mut u8` | reserve `len` bytes of input, 0 when it cannot be had | - |
//! | `omex_output_ptr() -> *const u8` | where the output of the last call is | - |
//! | `omex_output_len() -> usize` | how long it is | - |
//! | `omex_new() -> i32` | open an empty archive, to build one from scratch | nothing |
//! | `omex_open() -> i32` | open the archive in the input buffer | nothing |
//! | `omex_manifest() -> i32` | the entries of the archive | JSON |
//! | `omex_entry(ptr, len) -> i32` | the bytes of the entry at that location | the bytes |
//! | `omex_set_entry(ptr, len) -> i32` | a JSON header and the bytes behind it | nothing |
//! | `omex_remove_entry(ptr, len) -> i32` | remove the entry at that location | nothing |
//! | `omex_write(recompress) -> i32` | write the archive | the bytes |
//! | `omex_about() -> i32` | the version and the backend of this build | JSON |
//!
//! Nothing of the archive itself lives here: this crate is the buffers (`buffers.rs`), the calls
//! ([`calls`]) and the panic hook (`panic.rs`), and the archive is `pymetadata-core`.
//!
//! ```bash
//! cargo build -p pymetadata-wasm --release --target wasm32-unknown-unknown
//! ```

mod buffers;
pub mod calls;
mod panic;

use pymetadata_core::Archive;

use crate::calls::CallError;

/// The return code of a call which answered.
const OK: i32 = 0;

/// The return code of a call which failed, whose output names the kind of the failure.
const FAILED: i32 = 1;

/// Reserve `len` bytes for a page to write a request into, 0 when they cannot be had.
///
/// The input buffer is the module's and holds one request at a time, so a second allocation
/// discards the first. A page writes the bytes of an archive here before `omex_open`, and the
/// location or the header of the other calls, which take the address back as their argument.
/// `omex_alloc(0)` is how a page gives the buffer back: `omex_open` does it by itself, since the
/// archive holds its own copy of the bytes, and the other calls keep the request they were
/// given until the next one arrives.
#[unsafe(no_mangle)]
pub extern "C" fn omex_alloc(len: usize) -> *mut u8 {
    panic::install();
    buffers::alloc(len)
}

/// The address of the output of the last call, 0 when there is none.
///
/// The output lives until the next call publishes the next one, so a page which keeps it copies
/// it out of the memory of the instance first.
#[unsafe(no_mangle)]
pub extern "C" fn omex_output_ptr() -> *const u8 {
    buffers::output_ptr()
}

/// The length of the output of the last call in bytes, 0 when there is none.
#[unsafe(no_mangle)]
pub extern "C" fn omex_output_len() -> usize {
    buffers::output_len()
}

/// Open an empty archive, which a page then builds with `omex_set_entry` and `omex_write`.
///
/// This is `Archive()` of the python host: the archive holds the two entries the specification
/// requires, `.` and `./manifest.xml`, and no member at all. Without it a page could only change
/// an archive it already had, and would have to hand-write a seed zip in JavaScript to make one,
/// which is the work this module exists to remove.
///
/// The archive which was open before is dropped, as `omex_open` drops it. The input buffer is
/// left as it is, since this call reads no request, and nothing here can fail: the answer is
/// always 0 and the output is empty, as it is after an open which succeeded.
#[unsafe(no_mangle)]
pub extern "C" fn omex_new() -> i32 {
    panic::install();
    buffers::set_archive(Some(Archive::new()));
    buffers::publish(Vec::new());
    OK
}

/// Open the archive whose bytes are in the input buffer.
///
/// The archive which was open before is dropped either way: after this call the open archive is
/// the one of the input buffer, or none at all, and a page which reads the manifest after a
/// failed open is told that there is no archive rather than handed the entries of the one
/// before it. The input buffer is given back to the instance, since the archive holds the copy
/// of the bytes it reads lazily from.
#[unsafe(no_mangle)]
pub extern "C" fn omex_open() -> i32 {
    panic::install();
    let opened = buffers::with_input(calls::open);
    buffers::release_input();
    match opened {
        Ok(archive) => {
            buffers::set_archive(Some(archive));
            buffers::publish(Vec::new());
            OK
        }
        Err(error) => {
            buffers::set_archive(None);
            buffers::publish(error.response());
            FAILED
        }
    }
}

/// The entries of the open archive, as the JSON of
/// [`pymetadata_core::wire::ManifestResponse`].
///
/// This is also how a page reads `manifest.xml`: its bytes are written from the entries, so
/// `omex_entry` refuses them.
#[unsafe(no_mangle)]
pub extern "C" fn omex_manifest() -> i32 {
    on_archive(|archive| calls::manifest(archive))
}

/// The bytes of one entry of the open archive, inflated on the first ask and cached.
///
/// `ptr` is the address `omex_alloc` returned and `len` the number of bytes of the location
/// written into it, in UTF-8. A null pointer and a length of 0 are the empty location, which is
/// refused as every location outside of the archive is; anything else which is not a range of
/// the input buffer is `internal` and names both numbers.
#[unsafe(no_mangle)]
pub extern "C" fn omex_entry(ptr: *const u8, len: usize) -> i32 {
    panic::install();
    on_request(ptr, len, |location| {
        on_archive(|archive| calls::entry(archive, location))
    })
}

/// Add an entry with its bytes to the open archive, or replace the entry of the same location.
///
/// The request is the JSON of [`pymetadata_core::wire::SetEntryRequest`] and the `len` content
/// bytes behind it, see [`calls::set_entry`]. `ptr` and `len` are as for [`omex_entry`], with
/// the header and its content bytes in place of the location.
#[unsafe(no_mangle)]
pub extern "C" fn omex_set_entry(ptr: *const u8, len: usize) -> i32 {
    panic::install();
    on_request(ptr, len, |request| {
        on_archive(|archive| calls::set_entry(archive, request))
    })
}

/// Remove one entry and its bytes from the open archive.
///
/// A location the archive does not hold is `not_found`, see [`calls::remove_entry`]. `ptr` and
/// `len` are as for [`omex_entry`].
#[unsafe(no_mangle)]
pub extern "C" fn omex_remove_entry(ptr: *const u8, len: usize) -> i32 {
    panic::install();
    on_request(ptr, len, |location| {
        on_archive(|archive| calls::remove_entry(archive, location))
    })
}

/// Write the open archive, with its entries and its manifest, into the output buffer.
///
/// `recompress` of 0 copies every entry which was not replaced as the compressed blob it is,
/// which is the write a page makes after it changed one file; any other value deflates every
/// entry again, see [`calls::write`].
#[unsafe(no_mangle)]
pub extern "C" fn omex_write(recompress: i32) -> i32 {
    on_archive(|archive| calls::write(archive, recompress != 0))
}

/// What this build is: the version and the deflate backend, as JSON.
///
/// Returns 0, since there is nothing about this answer which can fail that would not have
/// failed the instantiation of the module.
#[unsafe(no_mangle)]
pub extern "C" fn omex_about() -> i32 {
    panic::install();
    buffers::publish(calls::about());
    OK
}

/// Panic on purpose, so that a test can watch the panic hook work in a real engine.
///
/// Only built with the `panic-probe` feature, which no default build and no release of the npm
/// package enables. The panic runs the hook of `panic.rs`, which publishes its sentence, and
/// then traps, which is the one path no native test can take: `catch_unwind` unwinds where
/// `wasm32-unknown-unknown` traps.
#[cfg(feature = "panic-probe")]
#[unsafe(no_mangle)]
pub extern "C" fn omex_panic_probe() -> i32 {
    panic::install();
    panic!("panic probe: a panic on purpose")
}

/// Run `f` on the open archive and publish what it answers.
///
/// The one place every export which needs an archive goes through, so there is one answer for a
/// request which arrives before `omex_open` and one shape for the output of all of them.
fn on_archive(f: impl FnOnce(&mut Archive) -> Result<Vec<u8>, CallError>) -> i32 {
    panic::install();
    let answer = buffers::with_archive(|archive| match archive {
        Some(archive) => f(archive),
        None => Err(calls::no_archive()),
    });
    match answer {
        Ok(bytes) => {
            buffers::publish(bytes);
            OK
        }
        Err(error) => fail(&error),
    }
}

/// Publish a failure and answer with its code.
fn fail(error: &CallError) -> i32 {
    buffers::publish(error.response());
    FAILED
}

/// Run `f` on the request a page wrote into the input buffer, or report that it is not there.
///
/// The one place the three exports which take a pointer and a length go through, so there is one
/// answer and one sentence for a pointer which is none of the input buffer. The request is the
/// buffer's own bytes ([`buffers::with_request`]), which is what lets those three exports be
/// safe functions: a page cannot name a range this host would read outside of the allocation it
/// was given, however it mangles the two numbers.
///
/// The request is also read to completion before any buffer is written: `f` answers with owned
/// data and the publish happens after, so no borrow of the input buffer is live across a write.
fn on_request(ptr: *const u8, len: usize, f: impl FnOnce(&[u8]) -> i32) -> i32 {
    if ptr.is_null() {
        if len == 0 {
            // the empty request, which every call reads as the empty location
            return f(&[]);
        }
        return fail(&CallError::Internal(format!(
            "a request of {len} bytes at the null pointer: omex_alloc answered 0, so the \
             instance never reserved them and nothing was written"
        )));
    }
    let (start, bytes) = buffers::input_bounds();
    buffers::with_request(ptr, len, f).unwrap_or_else(|| {
        fail(&CallError::Internal(format!(
            "a request of {len} bytes at {:#x} is not inside the input buffer, which is {bytes} \
             bytes at {start:#x}; write the request into the allocation of omex_alloc",
            ptr.addr()
        )))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use serde_json::{Value, json};

    use super::*;

    /// An archive of the corpus, as a page would hand it over.
    const COMP_MODELS: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../core/tests/data/omex/CompModels.omex"
    ));

    /// The gate of the tests which drive the exports.
    ///
    /// The module state is one per instance, and an instance of `wasm32-unknown-unknown` has one
    /// thread; a native test binary has one thread per test. So the tests below take this and
    /// run one at a time, which is the single-threaded module [`buffers`] documents.
    static GATE: Mutex<()> = Mutex::new(());

    /// Take the gate and start from a module state which holds nothing.
    ///
    /// Every test of this binary takes it, the ones of [`crate::calls`] included: a panic of any
    /// of them runs the hook, which publishes into the output buffer, and the hook is the
    /// process's once an export installed it.
    pub(crate) fn gate() -> MutexGuard<'static, ()> {
        // a test which panicked inside `catch_unwind` poisons nothing, and one which panicked
        // outside of it already failed: the state is reset here either way
        let guard = GATE.lock().unwrap_or_else(PoisonError::into_inner);
        buffers::set_archive(None);
        buffers::release_input();
        buffers::publish(Vec::new());
        guard
    }

    /// The output of the last call, as a page reads it.
    fn output() -> Vec<u8> {
        let ptr = omex_output_ptr();
        let len = omex_output_len();
        if ptr.is_null() || len == 0 {
            return Vec::new();
        }
        // SAFETY: the address and the length are the ones of the live output buffer.
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    }

    /// The output of the last call, as JSON.
    fn answer() -> Value {
        let bytes = output();
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "the output is no JSON ({error}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    }

    /// Write `bytes` into the input buffer, as a page does before a call.
    fn put(bytes: &[u8]) -> *mut u8 {
        let ptr = omex_alloc(bytes.len());
        assert!(!ptr.is_null(), "the allocation of {} bytes", bytes.len());
        // SAFETY: the address is the one of the allocation above, which is as long as `bytes`.
        unsafe { std::slice::from_raw_parts_mut(ptr, bytes.len()) }.copy_from_slice(bytes);
        ptr
    }

    /// The whole protocol: open an archive, read it, change it and write it back.
    #[test]
    fn the_exports_drive_an_archive_through_the_module_state() {
        let _gate = gate();
        put(COMP_MODELS);
        assert_eq!(omex_open(), OK);
        assert!(output().is_empty(), "an open publishes no output");

        assert_eq!(omex_manifest(), OK);
        let entries = answer()["entries"].as_array().expect("the entries").len();
        assert_eq!(entries, 6);

        let location = b"./README.md";
        let ptr = put(location);
        assert_eq!(omex_entry(ptr, location.len()), OK);
        assert_eq!(output().len(), 103);
        // a shorter length is the shorter location it names, and no read past the request
        assert_eq!(omex_entry(ptr, 2), FAILED);
        assert_eq!(answer()["kind"], json!("not_found"));

        // a null pointer is the empty location, which the core refuses as it refuses every
        // location which is not inside the archive
        assert_eq!(omex_entry(std::ptr::null(), 0), FAILED);
        assert_eq!(answer()["kind"], json!("refused"));

        let mut request = br#"{"location":"./a.csv","len":4}"#.to_vec();
        request.extend_from_slice(b"a,b\n");
        let ptr = put(&request);
        assert_eq!(omex_set_entry(ptr, request.len()), OK);
        assert!(output().is_empty());

        assert_eq!(omex_write(0), OK);
        let written = output();
        let mut reopened = Archive::from_bytes(&written).expect("a readable archive");
        assert_eq!(reopened.locations().count(), 7);
        assert_eq!(
            reopened.entry_bytes("./a.csv").expect("the entry"),
            b"a,b\n"
        );

        let ptr = put(b"./a.csv");
        assert_eq!(omex_remove_entry(ptr, 7), OK);
        assert_eq!(omex_write(1), OK);
        let reopened = Archive::from_bytes(&output()).expect("a readable archive");
        assert_eq!(reopened.locations().count(), 6);

        assert_eq!(omex_about(), OK);
        assert_eq!(answer()["backend"], json!("zlib-rs"));
        assert_eq!(answer()["version"], json!(env!("CARGO_PKG_VERSION")));
    }

    /// An archive built from scratch, which is the one thing `omex_open` cannot give a page.
    #[test]
    fn a_new_archive_is_set_written_and_read_back() {
        let _gate = gate();
        assert_eq!(omex_new(), OK);
        assert!(output().is_empty(), "a new archive publishes no output");
        assert_eq!(omex_manifest(), OK);
        let entries = answer()["entries"].as_array().expect("the entries").len();
        assert_eq!(entries, 2, "'.' and './manifest.xml' and nothing else");

        let mut request = br#"{"location":"model.xml","master":true,"len":8}"#.to_vec();
        request.extend_from_slice(b"<sbml/>\n");
        let ptr = put(&request);
        assert_eq!(omex_set_entry(ptr, request.len()), OK);

        assert_eq!(omex_write(0), OK);
        let mut written = Archive::from_bytes(&output()).expect("a readable archive");
        assert_eq!(
            written.locations().collect::<Vec<&str>>(),
            [".", "./manifest.xml", "./model.xml"]
        );
        assert_eq!(
            written.entry_bytes("./model.xml").expect("the entry"),
            b"<sbml/>\n"
        );

        // and it drops the archive which was open, as an open does
        put(COMP_MODELS);
        assert_eq!(omex_open(), OK);
        assert_eq!(omex_new(), OK);
        assert_eq!(omex_manifest(), OK);
        assert_eq!(
            answer()["entries"].as_array().expect("the entries").len(),
            2
        );
    }

    /// A call which needs an archive before one was opened is `internal`, not a trap.
    #[test]
    fn a_call_before_an_open_archive_is_internal() {
        let _gate = gate();
        assert_eq!(omex_manifest(), FAILED);
        assert_eq!(answer()["kind"], json!("internal"));
        assert!(
            answer()["message"]
                .as_str()
                .expect("a message")
                .contains("omex_open"),
            "{}",
            answer()
        );
        assert_eq!(omex_write(0), FAILED);
        assert_eq!(answer()["kind"], json!("internal"));
        let ptr = put(b"./README.md");
        assert_eq!(omex_entry(ptr, 11), FAILED);
        assert_eq!(answer()["kind"], json!("internal"));
    }

    /// A failed open leaves no archive open, and not the one before it.
    #[test]
    fn a_failed_open_leaves_no_archive_open() {
        let _gate = gate();
        put(COMP_MODELS);
        assert_eq!(omex_open(), OK);
        put(b"PK\x03\x04 and nothing else");
        assert_eq!(omex_open(), FAILED);
        assert_eq!(answer()["kind"], json!("malformed"));
        assert_eq!(omex_manifest(), FAILED);
        assert_eq!(answer()["kind"], json!("internal"));
        // and the bytes of the request are given back to the instance either way
        assert_eq!(omex_alloc(0), std::ptr::null_mut());
    }

    /// An allocation which cannot be served is the null pointer, and not an abort.
    #[test]
    fn an_allocation_a_page_cannot_have_is_the_null_pointer() {
        let _gate = gate();
        assert_eq!(omex_alloc(0), std::ptr::null_mut());
        assert_eq!(omex_alloc(usize::MAX), std::ptr::null_mut());
        // and the buffer of a request which was not served holds nothing
        assert_eq!(omex_open(), FAILED);
        assert_eq!(answer()["kind"], json!("malformed"));
        let ptr = omex_alloc(8);
        assert!(!ptr.is_null());
    }

    /// A request which is not inside the input buffer is reported and never read.
    #[test]
    fn a_request_outside_of_the_input_buffer_is_reported_and_not_read() {
        let _gate = gate();
        let ptr = put(b"./README.md");
        // every one of these names bytes the page never wrote: an address below the buffer, one
        // above it, the buffer with a length past its end, and a length no slice can have
        for (at, len) in [
            (ptr.wrapping_sub(1), 1),
            (ptr.wrapping_add(11), 1),
            (ptr, 12),
            (ptr, usize::MAX),
            (std::ptr::dangling::<u8>().cast_mut(), usize::MAX),
        ] {
            assert_eq!(omex_entry(at, len), FAILED, "{at:?} for {len}");
            assert_eq!(answer()["kind"], json!("internal"));
            assert!(
                answer()["message"]
                    .as_str()
                    .expect("a message")
                    .contains("not inside the input buffer"),
                "{}",
                answer()
            );
        }
        // a null pointer with a length is a page which ignored a failed allocation, and is told
        // that rather than the sentence above
        assert_eq!(omex_entry(std::ptr::null(), 8), FAILED);
        assert!(
            answer()["message"]
                .as_str()
                .expect("a message")
                .contains("omex_alloc answered 0"),
            "{}",
            answer()
        );
        // and the empty request is the empty location, which is no archive of this test
        assert_eq!(omex_entry(std::ptr::null(), 0), FAILED);
        assert_eq!(answer()["kind"], json!("internal"), "no archive is open");
    }

    /// A panic leaves the sentence of the panic in the output buffer.
    #[test]
    fn the_panic_hook_leaves_a_panic_output_behind() {
        let _gate = gate();
        panic::install();
        let caught = std::panic::catch_unwind(|| panic!("a bug of this build"));
        assert!(caught.is_err());
        assert_eq!(answer()["kind"], json!("panic"));
        assert!(
            answer()["message"]
                .as_str()
                .expect("a message")
                .contains("a bug of this build"),
            "{}",
            answer()
        );
    }
}
