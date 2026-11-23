# pymetadata-core: the OMEX archive in Rust

**Status:** implemented in 0.1.0. Issue
[pymetadata#99](https://github.com/matthiaskoenig/pymetadata/issues/99).
**Date:** 2026-11-23.
**Evidence:** `BENCHMARKS.md`, the numbers of the implementation;
`spike/RESULTS.md`, the measurements this design was accepted on.

## Why

`pymetadata.omex` is used wherever a COMBINE archive is read or written, in
pipelines and in frontends. Two measured results decide the design
(`BENCHMARKS.md`, 1.4 MB archive, fastest of 5 runs on a shared machine):

- `Omex.to_omex` costs 230 ms, because python re-deflates every entry on
  every write. A Rust writer which copies the untouched entries as
  still-compressed blobs costs 0.15 ms, including opening the archive
  (1520x). When both sides deflate every entry the core is 1.6x to 2.0x
  faster, so the gain is the copy and not the deflate.
- Reading only `manifest.xml` costs 197 microseconds in python and 78 in
  Rust through `Archive::from_bytes`, which copies its input (2.5x). A
  floor of 12 microseconds (17x) skips the copy and the pass which adds the
  members the manifest does not list; it is not an API call. Listing the
  contents of an archive is the request a frontend makes most.

A browser has no python at all, so the same core has to run as WebAssembly.

## What this builds

A new repository, `pymetadata-core`, holding a Rust workspace with the OMEX
archive: a reader, a writer, the manifest and the format table. It ships two
artifacts, a python wheel (`pymetadata-core`) and a wasm module with a
dependency-free JavaScript loader (`pymetadata-core` on npm).
The layout follows the sibling packages `pkpdutils-core` and `sbmlsim-core`.

`pymetadata` then consumes the wheel: `pymetadata.omex` becomes a thin python
layer over the extension (a hard swap, no pure-python fallback and no second
implementation). That change lands in the `pymetadata` repository after
`pymetadata-core` 0.1.0 is on PyPI, and is out of scope for this document
beyond the API contract in "The python layer".

Out of scope for 0.1.0: annotations, the identifiers.org registry, the
ontologies and the web services. The "lookup" half of issue #99 is a second
milestone and gets its own design document; the one in this document is the
archive.

## Architecture

### Lazy, in memory

`Archive::from_bytes(&[u8])` parses the end-of-central-directory record, the
central directory and `manifest.xml`, and nothing else. An entry is inflated
when a caller asks for it, and the inflated bytes are cached. Writing builds
a new zip and copies every untouched entry as the compressed blob it already
is, so "load, change one file, save" never re-deflates the rest.

The two rejected alternatives: inflating everything up front (simple, but it
pays the whole cost for the manifest-only request and makes a 1.4 MB archive
many MB resident), and an fs-backed native path with a separate wasm path
(best native numbers, two code paths forever, browser second class).

### The core owns the zip container

The `zip` crate cannot be used. It declares `getrandom` with the `wasm_js`
feature as a hard dependency for `wasm32`, `getrandom/wasm_js` pulls in
`wasm-bindgen`, and the module then carries six imports instead of none (see
`spike/RESULTS.md`). The core therefore reads and writes the container
itself: end-of-central-directory, central directory, local headers, the
`stored` and `deflate` methods, zip64 for the sizes and counts, and the
legacy ZipCrypto decryption. That is the same scope `zipfile` covers, and the
raw-copy writer needs the raw members anyway.

Dependencies of the core crate, all pure Rust and all import-free on
`wasm32-unknown-unknown`:

- `flate2` with the `zlib-rs` backend, for inflate and deflate. The spike
  measured `miniz_oxide` at 141 ms against python's 126 ms on the one path
  every writer takes, and `zlib-rs` at 71 ms.
- `crc32fast`, for the CRC32 every zip entry carries.
- `roxmltree`, for `manifest.xml`, as `sbmlsim-core`'s `crates/sbml` uses.
- `thiserror`, for the error type.
- `serde` and `serde_json`, for the JSON wire format of the wasm host.

No `std::fs` and no `std::io` in the core: a host hands it bytes and takes
bytes back. Malformed input is an `OmexError`, never a panic, because a panic
traps the wasm instance. No indexing of host-controlled data, no `unwrap` of
it.

### Crates

| crate | package | type | what it is |
|---|---|---|---|
| `crates/core` | `pymetadata-core` | rlib | the archive, the manifest, the formats, the zip container |
| `crates/python` | `pymetadata-python` | cdylib + rlib | the pyo3 binding, wheel `pymetadata-core`, module `pymetadata_core` |
| `crates/wasm` | `pymetadata-wasm` | cdylib + rlib | the raw C ABI host, no imports, `publish = false` |

No `abi` crate: the wire types are the manifest and an error, they are small,
and they live in `core::wire`. A separate crate is cheap to add later if the
lookup milestone needs one.

`crates/core/src`, one responsibility per module:

- `zip/spec.rs`: the constants of the format, declared once for the reader and
  the writer, so that the two cannot drift apart over a signature, a method, a
  flag bit or a sentinel.
- `zip/eocd.rs`, `zip/central.rs`, `zip/local.rs`: the container records.
- `zip/reader.rs`: the lazy reader, member lookup, raw and inflated bytes.
- `zip/writer.rs`: the writer, `stored`, `deflate` and raw copy.
- `zip/crypto.rs`: ZipCrypto decryption.
- `manifest.rs`: `Entry`, `Manifest`, parse and serialize `manifest.xml`.
- `location.rs`: `normalize_location`.
- `formats.rs`: the format URI table, `lookup_format`, `guess_format`. The
  table is generated from `pymetadata.omex.EntryFormat` by
  `scripts/formats_table.py`, so the two never drift; `guess_format` takes
  the suffix and the first 256 bytes of the file, because telling SBML,
  SED-ML, CellML and COPASI apart means looking at the root tag, and reading
  the file is the host's job.
- `archive.rs`: `Archive`, the public surface.
- `error.rs`: `OmexError`.
- `wire.rs`: the serde types of the JSON surface.

### The public surface of the core

```rust
pub struct Archive { /* lazy */ }

impl Archive {
    pub fn new() -> Self;
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OmexError>;
    pub fn from_bytes_with_password(bytes: &[u8], password: &[u8]) -> Result<Self, OmexError>;
    pub fn manifest(&self) -> &Manifest;
    pub fn locations(&self) -> impl Iterator<Item = &str>;
    pub fn entry(&self, location: &str) -> Option<&Entry>;
    pub fn entry_bytes(&mut self, location: &str) -> Result<&[u8], OmexError>;
    pub fn set_entry(&mut self, entry: Entry, bytes: Vec<u8>) -> Result<(), OmexError>;
    pub fn remove_entry(&mut self, location: &str) -> Result<Option<Entry>, OmexError>;
    pub fn to_bytes(&mut self) -> Result<Vec<u8>, OmexError>;
    pub fn to_bytes_recompressed(&mut self, level: u32) -> Result<Vec<u8>, OmexError>;
}

pub struct Entry { pub location: String, pub format: String, pub master: bool }

pub struct Manifest { /* ordered, keyed by location */ }

impl Manifest {
    pub fn parse(xml: &str) -> Result<Self, OmexError>;
    pub fn to_xml(&self) -> String;
    pub fn entries(&self) -> &[Entry];
    pub fn get(&self, location: &str) -> Option<&Entry>;
    pub fn add(&mut self, entry: Entry);
    pub fn remove(&mut self, location: &str) -> Option<Entry>;
}
```

`to_bytes` is the raw-copy write: an entry whose bytes were never replaced is
copied as the compressed member it was read as, with its CRC and its sizes.
`to_bytes_recompressed` deflates everything and is what a caller asks for
when the archive was built from scratch or must be rewritten at a chosen
level. Both regenerate `manifest.xml` from the manifest, and both keep the
entry order of the manifest.

Invariants the core keeps, all of them the ones `pymetadata` keeps today:

- Every location is normalized to a `./`-prefixed relative path. `.` is the
  archive itself.
- The manifest always holds `.` with the OMEX format URI and `./manifest.xml`
  with the manifest format URI.
- An entry outside the archive (`../`) and an absolute location are errors, not
  silently dropped files, and they are errors of every call which takes a
  location: `entry_bytes`, `set_entry` and `remove_entry` alike.
- A location present in the zip but absent from the manifest is kept and
  listed; this is what real archives from BioModels look like.

### The python layer

The wheel is `pymetadata-core`, the extension module `pymetadata_core`, built
with maturin and pyo3 (abi3). Layout: `sbmlsim-core`'s mixed package, because
the archive needs `Path`, a temporary directory and `from_url`, which belong
in python and not in the extension. So there is a root `pyproject.toml` with
`manifest-path = "crates/python/Cargo.toml"`, `python-source = "python"` and
`module-name = "pymetadata_core._core"`, and `python/pymetadata_core/` holds
`__init__.py`, `_core.pyi` and `py.typed`.

The extension exposes one class and two functions, and keeps the python
names:

```python
class Archive:
    def __init__(self) -> None: ...
    @staticmethod
    def from_bytes(data: bytes, password: bytes | None = None) -> Archive: ...
    @property
    def entries(self) -> list[tuple[str, str, bool]]: ...  # location, format, master
    @property
    def members(self) -> list[tuple[str, int, int, int]]: ...  # see below
    @property
    def declared_entries(self) -> list[tuple[str, str, bool]]: ...  # see below
    def has_bytes(self, location: str) -> bool: ...
    def manifest_xml(self) -> str: ...
    def entry_bytes(self, location: str) -> bytes: ...
    def set_entry(
        self, location: str, format: str, master: bool, data: bytes
    ) -> None: ...
    def remove_entry(self, location: str) -> bool: ...
    def to_bytes(self, recompress: bool = False, level: int = 9) -> bytes: ...


def normalize_location(location: str) -> str: ...
def guess_format(suffix: str, head: bytes = b"") -> str: ...
def lookup_format(format_key: str) -> str: ...
```

`members` is the cheap view of the source container: one tuple per non directory
member of the zip, in central directory order, with the normalized location, the
compressed and uncompressed size and the raw zip method number (0 stored, 8
deflate), read from the central directory without inflating or copying a byte.
It describes the container and nothing else: it is empty for `Archive()` and it
does not grow when `set_entry` stages content. `has_bytes(location)` is the
per-entry question which sees staged content: true for a member of the container
or a `set_entry`, false for a listed entry no member provides and for `.` and
`./manifest.xml`, and `OmexError` (refused) for a location outside of the
archive. Together they let `pymetadata` tell a dangling entry from a member, and
find the members the manifest does not list, without `entry_bytes` inflating.

`declared_entries` is the manifest as the container's `manifest.xml` declared it:
one tuple per `content` element in document order, with no entry invented for a
member the manifest does not list, no required entry prepended, and an entry
without a member kept. It is parsed once when the archive is opened and does not
change with `set_entry` or `remove_entry`; it is empty for `Archive()`. A
location the manifest declares twice is listed twice, as parsed, while `entries`
holds it once, the last declaration winning. A host
needs it next to `entries` because `entries` is the effective manifest, in which
a declared entry cannot be told from an invented one, and `./manifest.xml` is
never readable as bytes; `pymetadata` orders the declared entries first, in the
manifest's order, and the undeclared members after them, sorted.

Errors from the core are raised as `pymetadata_core.OmexError`, a subclass of
`ValueError`, so a caller which catches `ValueError` today keeps working.

Opening, inflating an entry, `set_entry` and writing run without the GIL, so
another thread can work while a large archive is written. An `Archive` is in use by one call at
a time: a second thread which uses it meanwhile gets
`pymetadata_core.ArchiveBusyError` (a `RuntimeError`) saying so, and the rule is
one `Archive` per thread.

What `pymetadata` does with it (the contract, implemented in that repository):
`pymetadata.omex.Omex` keeps its public API unchanged, which is
`from_omex`, `from_url`, `from_directory`, `to_omex`, `to_directory`,
`add_entry`, `remove_entry_for_location`, `get_path`, `entries_by_format`,
`is_omex`, `lookup_format`, `guess_format`, the context manager and
`Manifest`/`ManifestEntry` as pydantic models. The changes are internal:
`from_omex` hands the file's bytes to the extension instead of extracting
everything into the temporary directory, `get_path` writes the one entry it
is asked for into the temporary directory on first ask, and `to_omex` calls
`Archive.to_bytes`. `pymetadata` gains a runtime dependency on
`pymetadata-core` and stops being a pure-python wheel. `tests/test_omex.py`
is the acceptance gate for the swap and does not change.

### Differences from pymetadata

The core keeps what an archive says, where `pymetadata` rewrites it on the way
through its temporary directory. The hard swap changes these observable
behaviors, so `tests/test_omex.py` of `pymetadata` has to be updated for them,
and none of them is a regression of the core. Found by the cross-checks of
`python/tests/test_roundtrip.py` against `pymetadata` 0.8.3 on
`iCGB21FR.omex`.

- A manifest entry without a zip member (`./iCGB21FR_memote.html`, where the zip
  holds `iCGB21_memote.html`): the core keeps it as an entry without bytes,
  `pymetadata` drops it, as its manifest is built from the extracted files.
- Files the manifest does not list: the core keeps them in the order of the zip,
  `pymetadata` sorts them by location, after the listed entries.
- The format of `.` and `./manifest.xml`: the core keeps the one the file
  declares (`https://identifiers.org/...` in that archive), `pymetadata` always
  writes its own default (`http://identifiers.org/...`).
- Threads: an `Archive` shared between threads can raise `ArchiveBusyError` while
  another thread inflates an entry, sets an entry or writes it, where pure python `pymetadata`
  never did. One `Archive` per thread avoids it.

### The wasm host

`crates/wasm` is a `cdylib` for `wasm32-unknown-unknown` with a raw C ABI and
no imports, and `crates/wasm/tests/imports.rs` walks the sections of the
built module and asserts there is no import section, as the sibling test
does. Every export is a thin wrapper around a pure function of the core, so
the layer is tested natively.

Exports, each returning 0 for an answer and a nonzero code for an error, with
the payload in the output buffer:

```
omex_alloc(len) -> *mut u8
omex_output_ptr() -> *const u8
omex_output_len() -> usize
omex_new() -> i32               // an empty archive, as `Archive()` in python
omex_open() -> i32              // parse the input buffer as an archive
omex_manifest() -> i32          // JSON: the entries
omex_entry(ptr, len) -> i32     // the inflated bytes of one location
omex_set_entry(ptr, len) -> i32 // JSON header plus bytes
omex_remove_entry(ptr, len) -> i32
omex_write(recompress) -> i32   // the archive bytes
omex_about() -> i32             // JSON: version, backend
```

Error codes mirror the sibling. The core produces five, the kind strings of
`wire::kind`: `malformed`, `refused`, `unsupported`, `not_found` and
`password`, one per variant of `OmexError`. A `password` names a member whose
password is wrong or missing; its message carries the `refused` prefix every
other refusal carries, so the kind, and in python the class of the exception,
is what tells a member a caller may not read from every other refusal. Two more are host-level and the core never returns
them: `internal`, which the wasm host raises for a failure outside of the core
(a request before `omex_new` or `omex_open`, or a pointer and a length which
are not a range of the input buffer, which is where `omex_alloc` is and the
only place a request is read from), and `panic`, which its panic hook raises,
writing the message into the output buffer before the trap.

Two limitations of 0.1.0, both of them of the export list above and not of the
core. `omex_open` takes no password, so a ZipCrypto protected archive cannot be
opened in the browser, where the python host opens it with
`Archive.from_bytes(data, password)`. And the host sets no budget of its own, so
the browser inherits the 1 GiB inflate cap of `zip::MAX_INFLATED_SIZE`: a member
which declares more is `refused`, with its size and the limit in the message,
while a member under the cap which the instance cannot allocate aborts, and an
abort on `wasm32` traps. A page sees that trap as a `RuntimeError` with no
message of its own, the output buffer still holds whatever the call before it
wrote, and the instance is spent: the loader instantiates a new one rather than
calling into it again.

Five more differences between the two hosts, all of them of the export list and
none of them of the core, which `js/README.md` states for a page as well:

- `omex_set_entry` guesses an absent `format` from the suffix of the location
  and the first bytes of the content, where the python `set_entry` takes one.
  Python exposes the guess as `guess_format(suffix, head)`, so a python caller
  takes the suffix of the location itself.
- There is no export for the bytes of `manifest.xml`: they are written from the
  entries, so `omex_manifest` is how a page reads the manifest, where python
  also has `manifest_xml()`.
- `Archive.members`, `Archive.declared_entries` and `Archive.has_bytes` are
  python-only in 0.2.0; the wasm host does not export them.
- `normalize_location` and `lookup_format` are python-only. A page which needs
  a format URI by name carries the table itself.
- `omex_remove_entry` answers `not_found` for an entry which was not there,
  where `Archive.remove_entry` answers `False`. A location outside of the
  archive is `refused` by both, and by every other call which takes one.

The cargo feature `panic-probe`, off by default, adds an export which panics
on purpose. `js/test/loader.test.mjs` builds the module with it into its own
target directory to prove, in a real engine, that the hook's sentence is what
the loader reads after the trap; the default module exports exactly the eleven
names above and the memory their output is read in, twelve in all, which
`crates/wasm/tests/imports.rs` and the same test assert. The loader reports a
trap which left no message as the kind `trap` and every call after a trap as
`spent`; those two kinds are the loader's own, the core and the wasm host never
produce them.

`js/` ships `loader.mjs` with no dependency and no Node API, so it runs in a
browser, a Web Worker and Node. `js/build.mjs` builds the module, copies it
to `js/runtime/pymetadata-core.wasm` and prints the byte and gzipped sizes,
and `--check` fails when the module is missing or stale. Tests run with
`node --test js/test/*.test.mjs`. No worker pool: an archive is one object,
and sharding it over workers has no meaning.

## Benchmarks

Not a gate, as in the siblings. `crates/core/examples/omex_timing.rs` times
the four archives of `pymetadata/tests/data/omex/` with the protocol of the
spike (3 warm-up calls, 3 rounds of 20 calls, median of the round medians,
a checksum per cell, then the fastest of 5 runs), `scripts/python_timing.py`
repeats it against the installed `pymetadata`, and `BENCHMARKS.md` holds the
comparison and the section against the spike numbers. `bench/RESULTS.md`
holds the node and Chrome table, produced by `node js/bench.mjs` and
`js/bench.html`.

## Repository conventions

Copied from `pkpdutils-core` unless noted: `develop` is the default branch
and takes every change through a pull request, `rust-toolchain.toml` pins
1.100.0 with `rustfmt`, `clippy` and both wasm targets, edition 2024,
`rust-version = "1.100"`, `[workspace.lints.rust] missing_docs = "warn"`,
`Cargo.lock` committed and every gate `--locked`, path dependencies carrying
both `path` and `version`, `spike/` excluded from the workspace, dependabot
monthly with `versioning-strategy: lockfile-only`, release notes in
`release-notes/x.y.z.md` which the publish job reads before it uploads.
`default-members` excludes `crates/python`, and the wasm checks exclude it by
name, for the reason the sibling states: pyo3's abi3 floor would make a
recent python the price of `cargo test`.

Continuous integration, `.github/workflows/ci.yml`, with actions pinned to
exact versions:

- `checks`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings`,
  `cargo build -p pymetadata-wasm --release --locked --target
  wasm32-unknown-unknown`,
  `cargo test --locked`, `RUSTDOCFLAGS="-D warnings" cargo doc --locked
  --no-deps`. The release build of the module comes before the tests because
  `crates/wasm/tests/imports.rs` reads it and fails when it is missing or older
  than a source, a manifest or the lock, rather than skipping: a `cargo test`
  which never saw the module is no answer about its imports, and `cargo test`
  does not build another target by itself.
- `wasm`: `cargo check --locked --workspace --exclude pymetadata-python` for
  `wasm32-unknown-unknown` and `wasm32-wasip1`, with and without default
  features.
- `js`: the release wasm build, `cargo test -p pymetadata-wasm`, clippy,
  `node js/build.mjs`, `npm test`, `npm pack --dry-run`.
- `python`: fmt, clippy `-p pymetadata-python`, `cargo test -p
  pymetadata-python`, `maturin build --release --locked`, install the wheel,
  `pytest python/tests`, an sdist build.
- `ruff` and `ty`: the python layer and `scripts/`, with the versions
  `pymetadata` pins.

`.github/workflows/release.yml` builds the wheel matrix and the sdist,
publishes to PyPI with trusted publishing from the `pypi` environment on a
`v*` tag, then publishes the npm package with provenance, then the GitHub
release with the release-notes file as its body. Nothing goes to crates.io
in 0.1.0.

## Follow-ups

- **Move the input instead of copying it (lookup milestone).** `Archive::from_bytes`
  takes `&[u8]` and copies it into the zip reader, which owns a `Vec`, and the
  wasm host stages the request in its own buffer and releases it after
  `omex_open`, so an archive is copied once more than it needs to be. The copy
  is 0.047 ms of the 0.078 ms of the manifest-only call on the 1.4 MB archive
  (`BENCHMARKS.md`); the other 0.019 ms of the 0.066 ms gap to `manifest_owned`
  is the pass which adds the unlisted members, which stays. An
  `Archive::from_vec` which takes the staged `Vec`, used by the host, would
  reach about 0.031 ms, roughly 6x against python's 0.197 ms, and not the
  0.012 ms of the floor. It belongs with the host and its own review, and is not done.

## Risks

- **Hand-rolled zip.** The container is the one piece where a bug corrupts a
  user's archive. Mitigation: the four real archives of
  `pymetadata/tests/data/omex/` are the fixture corpus, every write is read
  back by python's `zipfile` in a test, and `zipfile`'s own output is read by
  the core in the same test, so both directions are pinned against the
  reference implementation.
- **A pure-python wheel is lost.** Accepted, the hard swap is the decision of
  issue #99. `pymetadata` gains a wheel matrix, and a platform without a
  wheel can no longer install it; the matrix of the sibling release workflow
  covers linux, macos and windows on x86_64 and arm64.
- **Encrypted archives.** `zipfile` reads ZipCrypto and not AES, so the core
  matches it and rejects AES with a clear error. Writing an encrypted archive
  is not supported today and stays unsupported.
- **zip64 and oddities.** Archives in the wild carry zip64 records, data
  descriptors and a mixed `stored`/`deflate` membership. All three are in
  scope for 0.1.0 because BioModels archives carry them.
