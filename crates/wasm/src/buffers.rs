//! The module state of this host: the input buffer, the output buffer and the open archive.
//!
//! A page writes one request into the input buffer, calls one export, reads one output and
//! calls the next export, which is why the three live here and not in the signature of an
//! export: an archive of 100 MB is not carried across the boundary twice per call, and a page
//! which wants two archives at a time instantiates the module twice.
//!
//! # One thread, one call at a time
//!
//! The module is single-threaded by construction. `wasm32-unknown-unknown` has no thread of its
//! own, a page which wants two cores runs one instance per Web Worker, and every instance has
//! a linear memory and therefore a module state of its own. The module also imports nothing
//! (`crates/wasm/tests/imports.rs`), so it cannot call out of itself and no export can be
//! re-entered while another call is in flight: the one way back into this module is an export,
//! and an export is only called from the outside, which is not running while a call of it is.
//! Every borrow below is taken inside one export, dropped before it returns and never handed
//! out, so there is never a second live borrow of the same static.
//!
//! That is also why there are three statics and not one struct: `omex_set_entry` reads its
//! request from the input buffer while it writes the archive, and `omex_entry` publishes an
//! output while its location points into the input buffer, so a single `&mut` over all three
//! would alias where three separate borrows do not.
//!
//! If a host ever did re-enter an export, two `&mut` to one static would exist and the
//! behaviour would be undefined; the loader of the npm package therefore holds one instance per
//! call site and never calls an export from inside a callback of another.
//!
//! The claim holds on `wasm32` and nowhere else. This crate also builds as a native `cdylib`
//! and an `rlib`, where a process has as many threads as it likes, and nothing here is
//! synchronized: a native host which called two exports from two threads would race, and the
//! module is not offered as a native library for that reason. The native tests of this crate
//! are the one native caller there is, and they take the gate of `crate::tests` so that the
//! tests which drive an export, and any panic of one of them, run one at a time.

use std::cell::UnsafeCell;

use pymetadata_core::Archive;

/// One piece of module state, reached through a raw pointer.
///
/// `static mut` is not an option: edition 2024 forbids a reference to one, and a raw pointer
/// into an [`UnsafeCell`] says the same thing without the lint. The `Sync` bound a static needs
/// is the one claim of this type, and the module doc above is why it holds.
struct Slot<T>(UnsafeCell<T>);

// SAFETY: the module is single-threaded, see the module doc. On `wasm32-unknown-unknown`, which
// is the target this crate is built for, there is one thread and nothing of it is reachable from
// two at once. The claim is not bounded by a `cfg`, because the one other place this code runs is
// the native test binary of this crate, which is a caller and not another target: its tests take
// the gate of `crate::tests` and drive the exports one at a time.
unsafe impl<T> Sync for Slot<T> {}

/// The bytes a page wrote into the allocation of `omex_alloc`, read by the call after it.
static INPUT: Slot<Vec<u8>> = Slot(UnsafeCell::new(Vec::new()));

/// The output of the last call, which a page reads at `omex_output_ptr`.
static OUTPUT: Slot<Vec<u8>> = Slot(UnsafeCell::new(Vec::new()));

/// The archive `omex_open` opened, `None` before the first open and after a failed one.
static ARCHIVE: Slot<Option<Archive>> = Slot(UnsafeCell::new(None));

/// Reserve `len` bytes of input and give back the address a page writes them to.
///
/// The address is null when `len` is 0 and when the instance cannot grow by that much, which is
/// what a page checks for: nothing is written and no call which follows has a request. An
/// allocation of 0 is no allocation, so a page which reserves nothing is told so rather than
/// handed the dangling address of an empty buffer.
///
/// The input buffer is the module's, so a second call to this discards the request of the first.
pub(crate) fn alloc(len: usize) -> *mut u8 {
    // SAFETY: the module is single-threaded and no other borrow of `INPUT` is live: every one of
    // them is taken and dropped inside `with_input`, which runs no code of a page.
    let input = unsafe { &mut *INPUT.0.get() };
    input.clear();
    if len == 0 || input.try_reserve_exact(len).is_err() {
        input.shrink_to_fit();
        return std::ptr::null_mut();
    }
    // the reservation above is what makes this grow without ever asking the allocator again, so
    // an instance which cannot grow answers with the null pointer above and never aborts here
    input.resize(len, 0);
    input.as_mut_ptr()
}

/// Hand the bytes of the input buffer to `f`, for the duration of the call and no longer.
///
/// The closure is what keeps the borrow inside one export: it takes the bytes, answers with
/// owned data and cannot leak the slice. `f` must not call [`alloc`] or [`release_input`],
/// which would invalidate the very slice it is reading; nothing of this crate does.
pub(crate) fn with_input<T>(f: impl FnOnce(&[u8]) -> T) -> T {
    // SAFETY: the module is single-threaded and no `&mut` of `INPUT` is live, see `alloc`.
    let input = unsafe { &*INPUT.0.get() };
    f(input)
}

/// Hand the `len` bytes at `ptr` to `f`, or answer `None` when they are no range of the input.
///
/// This is what makes the three exports which take a pointer and a length safe functions: a
/// page writes its request into the allocation of [`alloc`], which is this buffer, so a pointer
/// and a length which are not a range of it are no request at all and are reported instead of
/// read. The bytes are then taken from the buffer by index and not from the address of the page,
/// so there is no slice built from a pointer a page chose, and a length which does not match
/// what it wrote can at worst read the rest of its own request.
///
/// As [`with_input`], the closure bounds the borrow, and `f` must not call [`alloc`] or
/// [`release_input`].
pub(crate) fn with_request<T>(ptr: *const u8, len: usize, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
    // SAFETY: as `with_input`.
    let input = unsafe { &*INPUT.0.get() };
    let at = ptr.addr().checked_sub(input.as_ptr().addr())?;
    let end = at.checked_add(len)?;
    Some(f(input.get(at..end)?))
}

/// Where the input buffer is and how many bytes it holds, as a page sees it.
///
/// For the sentence of a request which is not inside it; a page which needs the address itself
/// has it from [`alloc`].
pub(crate) fn input_bounds() -> (usize, usize) {
    // SAFETY: as `with_input`.
    let input = unsafe { &*INPUT.0.get() };
    (input.as_ptr().addr(), input.len())
}

/// Give the bytes of the input buffer back to the instance.
///
/// `omex_open` calls this: the archive holds a copy of the bytes it was opened from, so keeping
/// the request as well would make a browser tab carry every archive it opens twice.
pub(crate) fn release_input() {
    // SAFETY: as `alloc`.
    let input = unsafe { &mut *INPUT.0.get() };
    input.clear();
    input.shrink_to_fit();
}

/// Publish the output of a call and drop the one before it.
///
/// Every export publishes exactly one output, an answer or the JSON of an error, so a page
/// which reads a length of 0 after a call which failed is a host that could not say anything.
pub(crate) fn publish(bytes: Vec<u8>) {
    // SAFETY: the module is single-threaded and no borrow of `OUTPUT` is live: `output_ptr` and
    // `output_len` take one and drop it, and an export builds its output before it publishes it.
    // The panic hook publishes as well, from inside a call which holds a borrow of `ARCHIVE` or
    // a slice of `INPUT`, which is why those are statics of their own.
    unsafe { *OUTPUT.0.get() = bytes };
}

/// The address of the output of the last call, 0 when there is none.
pub(crate) fn output_ptr() -> *const u8 {
    // SAFETY: as `publish`.
    let output = unsafe { &*OUTPUT.0.get() };
    if output.is_empty() {
        return std::ptr::null();
    }
    output.as_ptr()
}

/// The length of the output of the last call in bytes, 0 when there is none.
pub(crate) fn output_len() -> usize {
    // SAFETY: as `publish`.
    unsafe { (*OUTPUT.0.get()).len() }
}

/// Keep `archive` as the one every call after this works on, and drop the one before it.
pub(crate) fn set_archive(archive: Option<Archive>) {
    // SAFETY: the module is single-threaded and no borrow of `ARCHIVE` is live: the only one is
    // taken inside `with_archive`, and this is never called from there.
    unsafe { *ARCHIVE.0.get() = archive };
}

/// Hand the open archive to `f`, or `None` when no open succeeded yet.
///
/// As [`with_input`], the closure is what bounds the borrow. `f` must not call [`set_archive`]
/// or this function again; nothing of this crate does.
pub(crate) fn with_archive<T>(f: impl FnOnce(Option<&mut Archive>) -> T) -> T {
    // SAFETY: as `set_archive`.
    let archive = unsafe { &mut *ARCHIVE.0.get() };
    f(archive.as_mut())
}
