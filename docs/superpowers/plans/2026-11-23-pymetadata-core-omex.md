# pymetadata-core OMEX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `pymetadata-core`, a Rust workspace which reads and writes COMBINE archives lazily, and ship it as a python wheel and a zero-import wasm module.

**Architecture:** One `core` crate owning the zip container, the manifest and the format table, with no `std::fs` and no `std::io`, plus two host crates: a pyo3 binding (`crates/python`, wheel `pymetadata-core`) and a raw C ABI wasm host (`crates/wasm`, no imports). An archive parses its central directory and `manifest.xml` only, inflates an entry on demand and caches it, and writes by copying untouched entries as compressed blobs.

**Tech Stack:** Rust 1.100.0, edition 2024, `flate2` with the `zlib-rs` backend, `crc32fast`, `roxmltree`, `thiserror`, `serde`/`serde_json`, pyo3 abi3 with maturin, `wasm32-unknown-unknown`, node 20+ for the JavaScript loader.

**Spec:** `docs/design/omex-core.md` (evidence: `spike/RESULTS.md`)

## Global Constraints

- Repository `/home/mkoenig/git/pymetadata-core`, default branch `develop`, every change through a pull request.
- `rust-toolchain.toml`: channel `1.100.0`, components `rustfmt` and `clippy`, targets `wasm32-wasip1` and `wasm32-unknown-unknown`.
- `[workspace.package]`: `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.100"`, `license = "MIT"`, `authors = ["Matthias König"]`, `repository = "https://github.com/matthiaskoenig/pymetadata-core"`, `readme = "README.md"`, `keywords = ["combine", "omex", "sbml", "sedml", "wasm"]`, `categories = ["science", "wasm"]`.
- `[workspace.lints.rust] missing_docs = "warn"`, and every crate carries `[lints] workspace = true`. Every public item has a doc comment.
- `default-members` is every crate except `crates/python`; `--workspace --exclude pymetadata-python` is how the wasm checks run.
- `exclude = ["spike"]`: the spike package stays at the repository root and is never a member.
- `Cargo.lock` is committed and every gate runs `--locked`. Path dependencies carry both `path` and `version = "0.1.0"`.
- No `std::fs` and no `std::io` under `crates/core/src` and `crates/wasm/src`. No panic on host input: malformed bytes are an `OmexError`. No indexing or `unwrap` of host-controlled data.
- The fixture corpus is the six archives of `/home/mkoenig/git/pymetadata/tests/data/omex/`, copied into `crates/core/tests/data/omex/` (`.gitattributes`: `crates/core/tests/data/** -text`).
- No em dash anywhere, in code, comments, docs or commit messages.
- Commit messages are conventional (`feat:`, `test:`, `docs:`, `ci:`) with no attribution lines.

## Review Focus

Each line names an input the spec implies and the behavior a person expects. The test is added to the task in brackets.

1. An archive whose zip holds a file absent from `manifest.xml` lists and reads that file instead of hiding it (BioModels archives look like this) [Task 5].
2. An archive written by the core and read back by python's `zipfile` has the same members, CRCs and bytes, and the reverse direction too [Task 6].
3. A `manifest.xml` whose `location` is `.\models\model.xml`, `models/model.xml` or `./models/model.xml` ends up as one and the same entry [Task 2].
4. A zip with a zip64 end-of-central-directory record, or a member with a data descriptor and zero sizes in the local header, reads correctly instead of returning truncated bytes [Task 3].
5. An AES-encrypted member is refused with a message naming AES, rather than returning inflated garbage [Task 4].

---

### Task 1: Workspace, error type, locations and formats

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `.gitattributes`, `README.md`, `LICENSE` (MIT, Matthias König)
- Create: `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/core/src/error.rs`, `crates/core/src/location.rs`, `crates/core/src/formats.rs`
- Create: `scripts/formats_table.py`, `crates/core/tests/data/formats.json`
- Test: `crates/core/src/location.rs` (unit tests), `crates/core/tests/formats_parity.rs`

**Interfaces:**
- Produces: `pymetadata_core::OmexError` (`thiserror`, variants `Malformed(String)`, `Refused(String)`, `Unsupported(String)`, `NotFound(String)`), `normalize_location(&str) -> Result<String, OmexError>`, `lookup_format(&str) -> &'static str`, `guess_format(&str, &[u8]) -> &'static str`, `UNKNOWN_FORMAT: &str`.

- [ ] **Step 1: Write the failing location tests** in `crates/core/src/location.rs`

```rust
#[test] fn normalizes_plain_relative() { assert_eq!(normalize_location("models/model.xml").unwrap(), "./models/model.xml"); }
#[test] fn normalizes_backslashes() { assert_eq!(normalize_location(".\\models\\model.xml").unwrap(), "./models/model.xml"); }
#[test] fn keeps_dot_prefix() { assert_eq!(normalize_location("./manifest.xml").unwrap(), "./manifest.xml"); }
#[test] fn archive_itself_is_dot() { assert_eq!(normalize_location(".").unwrap(), "."); }
#[test] fn drops_empty_segments() { assert_eq!(normalize_location("./models//model.xml").unwrap(), "./models/model.xml"); }
#[test] fn refuses_empty() { assert!(normalize_location("").is_err()); }
#[test] fn refuses_absolute() { assert!(normalize_location("/model.xml").is_err()); }
#[test] fn refuses_drive_letter() { assert!(normalize_location("C:/model.xml").is_err()); }
#[test] fn refuses_parent() { assert!(normalize_location("../model.xml").is_err()); }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pymetadata-core --locked`
Expected: FAIL, `normalize_location` not found.

- [ ] **Step 3: Write the workspace and crate manifests, `rust-toolchain.toml`, `.gitignore` (`target/`, `__pycache__/`, `js/runtime/`, `js/node_modules/`, `.superpowers/`), `.gitattributes`, `LICENSE` and a README with a "Checks" section**

Mirror `/home/mkoenig/git/pkpdutils-core`'s root files, with the Global Constraints values. `crates/core` dependencies: `flate2` with `default-features = false`, features `["zlib-rs"]`; `crc32fast` with `default-features = false`; `roxmltree` with `default-features = false`, features `["std"]`; `thiserror`; `serde` with `derive`; `serde_json`.

- [ ] **Step 4: Implement `error.rs` and `normalize_location` in `location.rs`**

`normalize_location` follows `pymetadata.omex.normalize_location` (`/home/mkoenig/git/pymetadata/src/pymetadata/omex.py:361`): backslashes to slashes, reject empty, absolute, a `^[A-Za-z]:` drive prefix and any `..` segment, drop empty and `.` segments, return `"."` for no segments and `./` plus the joined segments otherwise.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p pymetadata-core --locked`
Expected: PASS.

- [ ] **Step 6: Write `scripts/formats_table.py`**

It imports `pymetadata.omex`, and writes two files: `crates/core/src/formats.rs` with a generated `const FORMATS: &[(&str, &str)]` of every `EntryFormat` member (the lowercased member name as the key, the URI as the value) plus the generated `XML_ROOT_FORMATS` table from `pymetadata.omex._XML_ROOT_FORMATS` and the suffix map, and `crates/core/tests/data/formats.json` with the same pairs as JSON. The file starts with a "generated, do not edit" comment naming the script.

- [ ] **Step 7: Write the failing formats parity test** in `crates/core/tests/formats_parity.rs`

```rust
#[test] fn every_key_of_pymetadata_resolves_to_the_same_uri() { /* read tests/data/formats.json, assert lookup_format(key) == uri for every pair */ }
#[test] fn unknown_key_is_the_unknown_media_type() { assert_eq!(lookup_format("nosuchformat"), "https://purl.org/NET/mediatypes/application/x.unknown"); }
#[test] fn guesses_sbml_from_the_root_tag() { assert_eq!(guess_format("xml", b"<?xml version='1.0'?><sbml xmlns='http://www.sbml.org/sbml/level3/version2/core'>"), "http://identifiers.org/combine.specifications/sbml.level-3.version-2"); }
#[test] fn guesses_csv_from_the_suffix() { assert_eq!(guess_format("csv", b""), "https://purl.org/NET/mediatypes/text/csv"); }
#[test] fn guesses_unknown_for_an_unknown_suffix() { assert_eq!(guess_format("zzz", b""), "https://purl.org/NET/mediatypes/application/x.unknown"); }
```

- [ ] **Step 8: Run the tests to verify they fail, then run the generator and implement the lookups**

Run: `cargo test -p pymetadata-core --locked` (FAIL), then
`cd /home/mkoenig/git/pymetadata && uv run python ../pymetadata-core/scripts/formats_table.py`,
then implement `lookup_format` (case-insensitive key over the generated table, `UNKNOWN_FORMAT` otherwise) and `guess_format` (for the `xml` suffix match the root-tag table against the head bytes first, then fall back to the suffix) in `formats.rs`.

- [ ] **Step 9: Run every gate**

Run: `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat: the workspace, the error type, locations and the format table"
```

---

### Task 2: The manifest

**Files:**
- Create: `crates/core/src/manifest.rs`
- Create: `crates/core/tests/data/omex/` (copy the six archives from `/home/mkoenig/git/pymetadata/tests/data/omex/`)
- Test: `crates/core/src/manifest.rs` (unit tests), `crates/core/tests/manifest_parity.rs`

**Interfaces:**
- Consumes: `normalize_location`, `OmexError`, `lookup_format` (Task 1).
- Produces: `Entry { location: String, format: String, master: bool }` (`serde::Serialize`, `Deserialize`), `Manifest` with `parse(&str) -> Result<Manifest, OmexError>`, `to_xml(&self) -> String`, `entries(&self) -> &[Entry]`, `get(&self, &str) -> Option<&Entry>`, `add(&mut self, Entry)`, `remove(&mut self, &str) -> Option<Entry>`, `Manifest::default()` holding `.` and `./manifest.xml`.

- [ ] **Step 1: Write the failing tests** in `crates/core/src/manifest.rs`

```rust
#[test] fn default_holds_the_two_required_entries() { let m = Manifest::default(); assert_eq!(m.entries().len(), 2); assert_eq!(m.get(".").unwrap().format, "http://identifiers.org/combine.specifications/omex"); assert_eq!(m.get("./manifest.xml").unwrap().format, "http://identifiers.org/combine.specifications/omex-manifest"); }
#[test] fn parses_locations_in_every_spelling_to_one_entry() { /* a manifest with ".\\models\\model.xml", "models/model.xml" and "./models/model.xml" parses to one entry "./models/model.xml" */ }
#[test] fn parses_master() { /* master="true" is true, master="1" is true, absent is false, master="false" is false */ }
#[test] fn refuses_a_wrong_root_element() { assert!(Manifest::parse("<other/>").is_err()); }
#[test] fn refuses_content_without_location_or_format() { /* both are Malformed */ }
#[test] fn roundtrips_through_xml() { /* parse(to_xml()) equals the manifest */ }
#[test] fn escapes_the_attributes() { /* a location with & and " is escaped and parses back byte for byte */ }
#[test] fn add_replaces_the_entry_for_a_location() { /* add twice, len unchanged, format is the second one */ }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pymetadata-core --locked manifest`
Expected: FAIL, `Manifest` not found.

- [ ] **Step 3: Implement `Entry` and `Manifest` in `crates/core/src/manifest.rs`**

`parse` with `roxmltree`: the root element must be `omexManifest`, every `content` child gives location (normalized), format and master. Order is the document order, with the two required entries first when absent. `to_xml` writes the header `<?xml version='1.0' encoding='utf-8'?>`, the root with `xmlns="http://identifiers.org/combine.specifications/omex-manifest"`, and one `content` element per entry with `master="true"` only when master, escaping `&`, `<`, `>` and `"` in the attribute values. Keep the entries in a `Vec<Entry>` plus a `HashMap<String, usize>` index, as `pymetadata.omex.Manifest` keeps its `_entries_dict`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pymetadata-core --locked manifest`
Expected: PASS.

- [ ] **Step 5: Write the parity test** in `crates/core/tests/manifest_parity.rs`

For each of the six archives, read the `manifest.xml` member with `std::fs` plus a minimal inline zip read of the stored or deflated member (test code may use `std::io`), parse it and assert the locations and formats equal the JSON written by `scripts/manifest_parity.py`, which dumps `Omex.from_omex(...).manifest.model_dump()` for the same archives.

- [ ] **Step 6: Run the parity test**

Run: `cd /home/mkoenig/git/pymetadata && uv run python ../pymetadata-core/scripts/manifest_parity.py`, then `cargo test -p pymetadata-core --locked --test manifest_parity`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: the manifest of a COMBINE archive"
```

---

### Task 3: The zip reader

**Files:**
- Create: `crates/core/src/zip/mod.rs`, `crates/core/src/zip/eocd.rs`, `crates/core/src/zip/central.rs`, `crates/core/src/zip/local.rs`, `crates/core/src/zip/reader.rs`
- Test: `crates/core/src/zip/reader.rs` (unit tests), `crates/core/tests/zip_reader.rs`

**Interfaces:**
- Consumes: `OmexError` (Task 1).
- Produces: `zip::Member { name: String, method: u16, crc32: u32, compressed_size: u64, uncompressed_size: u64, header_offset: u64, flags: u16 }`, `zip::Reader::new(&[u8]) -> Result<Reader, OmexError>`, `Reader::members(&self) -> &[Member]`, `Reader::raw(&self, &Member) -> Result<&[u8], OmexError>` (the compressed bytes as stored), `Reader::inflate(&self, &Member) -> Result<Vec<u8>, OmexError>`.

- [ ] **Step 1: Write the failing tests** in `crates/core/tests/zip_reader.rs`

```rust
#[test] fn reads_the_members_of_every_fixture() { /* for each of the six archives: members() is not empty, every name is unique, manifest.xml is among them */ }
#[test] fn inflate_matches_python_zipfile() { /* assert the sha256 of every member against tests/data/members.json, written by scripts/members_oracle.py */ }
#[test] fn reads_a_stored_member() { /* a hand-built zip with one stored member returns its bytes */ }
#[test] fn reads_zip64_sizes() { /* a hand-built zip with a zip64 eocd record and zip64 extra fields reads the full sizes */ }
#[test] fn reads_a_member_with_a_data_descriptor() { /* local header sizes are zero and bit 3 of the flags is set: the central directory sizes win */ }
#[test] fn refuses_a_truncated_archive() { /* Malformed, not a panic */ }
#[test] fn refuses_a_bad_crc() { /* a flipped byte in a stored member is Malformed */ }
#[test] fn refuses_an_unsupported_method() { /* method 12 (bzip2) is Unsupported and names the method */ }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pymetadata-core --locked --test zip_reader`
Expected: FAIL, `zip::Reader` not found.

- [ ] **Step 3: Write `scripts/members_oracle.py` and run it**

It reads the six archives with `zipfile`, and writes `crates/core/tests/data/members.json`: per archive a list of `{name, method, crc32, size, sha256}` of the inflated bytes.

- [ ] **Step 4: Implement the reader**

`eocd.rs`: find the end-of-central-directory signature by scanning back from the end over at most 64 kB, read the count and the central-directory offset, and follow the zip64 locator and record when the fields are `0xffff`/`0xffffffff`. `central.rs`: parse the central directory entries, including the zip64 extended-information extra field (header id `0x0001`). `local.rs`: parse the local header to find where the data starts, and take the sizes from the central directory. `reader.rs`: `raw` slices the compressed bytes, `inflate` matches the method (0 stored, 8 deflate through `flate2::read::DeflateDecoder`), verifies the CRC with `crc32fast` and the uncompressed size, and returns `Unsupported` for every other method. Every length is checked against the buffer before it is used.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p pymetadata-core --locked --test zip_reader`
Expected: PASS.

- [ ] **Step 6: Run every gate and commit**

```bash
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked
git add -A
git commit -m "feat: a lazy zip reader for the archive container"
```

---

### Task 4: ZipCrypto decryption

**Files:**
- Create: `crates/core/src/zip/crypto.rs`
- Modify: `crates/core/src/zip/reader.rs` (a password on the reader)
- Test: `crates/core/tests/zip_crypto.rs`

**Interfaces:**
- Consumes: `zip::Reader`, `OmexError` (Task 3).
- Produces: `Reader::new_with_password(&[u8], &[u8]) -> Result<Reader, OmexError>`, and `inflate` decrypting an encrypted member before it inflates.

- [ ] **Step 1: Write `scripts/crypto_fixture.py` and the failing tests**

`zipfile` cannot write an encrypted archive, so the two fixtures are generated once and committed: `crates/core/tests/data/encrypted.zip` with `zip -e` (password `test1234`) and `crates/core/tests/data/aes.zip` with `7z a -p -mem=AES256`. The script regenerates them and exits nonzero with the missing tool named when `zip` or `7z` is absent; the tests read the committed files, so no gate needs either tool. Tests in `crates/core/tests/zip_crypto.rs`:

```rust
#[test] fn reads_a_zipcrypto_member_with_the_right_password() { /* the inflated bytes equal the plaintext fixture */ }
#[test] fn refuses_a_wrong_password() { /* Refused, and the message says the password */ }
#[test] fn refuses_an_encrypted_member_without_a_password() { /* Refused */ }
#[test] fn refuses_aes() { /* Unsupported, and the message names AES */ }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pymetadata-core --locked --test zip_crypto`
Expected: FAIL.

- [ ] **Step 3: Implement ZipCrypto in `crypto.rs`**

The three keys of the traditional PKWARE cipher, initialized from the password and updated with CRC32 table steps; the 12-byte encryption header is decrypted first and its last byte is checked against the high byte of the CRC (or of the mod time when bit 3 is set). AES (the `0x9901` extra field, method 99) returns `Unsupported`.

- [ ] **Step 4: Run the tests to verify they pass, then commit**

```bash
cargo test -p pymetadata-core --locked --test zip_crypto
git add -A
git commit -m "feat: read password protected archives with ZipCrypto"
```

---

### Task 5: The archive

**Files:**
- Create: `crates/core/src/archive.rs`
- Modify: `crates/core/src/lib.rs` (the public re-exports)
- Test: `crates/core/src/archive.rs` (unit tests), `crates/core/tests/archive.rs`

**Interfaces:**
- Consumes: `Manifest`, `Entry` (Task 2), `zip::Reader` (Task 3, 4), `guess_format`, `OmexError` (Task 1).
- Produces: `Archive` with `new()`, `from_bytes(&[u8])`, `from_bytes_with_password(&[u8], &[u8])`, `manifest(&self) -> &Manifest`, `locations(&self) -> impl Iterator<Item = &str>`, `entry(&self, &str) -> Option<&Entry>`, `entry_bytes(&mut self, &str) -> Result<&[u8], OmexError>`, `set_entry(&mut self, Entry, Vec<u8>) -> Result<(), OmexError>`, `remove_entry(&mut self, &str) -> Option<Entry>`.

- [ ] **Step 1: Write the failing tests** in `crates/core/tests/archive.rs`

```rust
#[test] fn opens_every_fixture_and_lists_its_entries() { /* for each archive: manifest().entries() is not empty and holds "." and "./manifest.xml" */ }
#[test] fn lists_a_zip_member_absent_from_the_manifest() { /* a hand-built archive whose manifest omits ./extra.txt: locations() holds "./extra.txt", its format is the guess from the suffix, and entry_bytes returns it */ }
#[test] fn entry_bytes_inflates_once() { /* two calls return the same slice content; the second does not re-inflate (assert through a counter in a unit test in archive.rs) */ }
#[test] fn entry_bytes_of_an_unknown_location_is_not_found() { /* NotFound */ }
#[test] fn set_entry_adds_and_replaces() { /* set a new location: it is in the manifest and readable; set it again: the bytes are the second ones */ }
#[test] fn set_entry_normalizes_the_location() { /* "models\\m.xml" lands as "./models/m.xml" */ }
#[test] fn remove_entry_removes_from_the_manifest_and_the_content() { /* entry_bytes is NotFound afterwards */ }
#[test] fn remove_entry_refuses_the_required_entries() { /* removing "." or "./manifest.xml" returns None and leaves the manifest intact */ }
#[test] fn new_is_an_empty_archive_with_the_two_required_entries() { /* manifest().entries().len() == 2 */ }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pymetadata-core --locked --test archive`
Expected: FAIL, `Archive` not found.

- [ ] **Step 3: Implement `Archive` in `archive.rs`**

It owns the input bytes (`Vec<u8>`), the `zip::Reader` over them, the `Manifest`, a `HashMap<String, Content>` where `Content` is `Member(usize)`, `Inflated(Vec<u8>)` or `Replaced(Vec<u8>)`, and the cache. `from_bytes` reads the members, parses `./manifest.xml` into the manifest, adds an entry for every member absent from the manifest and keeps the manifest order. The format of such an entry is `guess_format(suffix, &[])`, the suffix alone: guessing from the root tag would inflate the member, and nothing may be inflated before a caller asks for it. `entry_bytes` inflates a `Member` once, stores it as `Inflated` and returns the slice.

- [ ] **Step 4: Run the tests to verify they pass, then commit**

```bash
cargo test -p pymetadata-core --locked
git add -A
git commit -m "feat: the lazy COMBINE archive"
```

---

### Task 6: The zip writer

**Files:**
- Create: `crates/core/src/zip/writer.rs`
- Modify: `crates/core/src/archive.rs` (`to_bytes`, `to_bytes_recompressed`)
- Test: `crates/core/tests/write.rs`, `crates/core/tests/data/` (no new fixture), `python/tests/test_roundtrip.py` is Task 8

**Interfaces:**
- Consumes: `Archive` (Task 5), `zip::Reader` (Task 3).
- Produces: `zip::Writer::new()`, `Writer::add_stored(&mut self, &str, &[u8])`, `Writer::add_deflated(&mut self, &str, &[u8], u32)`, `Writer::add_raw(&mut self, &str, &Member, &[u8])`, `Writer::finish(self) -> Vec<u8>`; `Archive::to_bytes(&mut self) -> Result<Vec<u8>, OmexError>`, `Archive::to_bytes_recompressed(&mut self, level: u32) -> Result<Vec<u8>, OmexError>`.

- [ ] **Step 1: Write the failing tests** in `crates/core/tests/write.rs`

```rust
#[test] fn written_archive_reads_back_with_the_same_entries() { /* for each fixture: to_bytes, Archive::from_bytes of the result, the manifests are equal and every entry's bytes are equal */ }
#[test] fn untouched_entries_are_copied_as_blobs() { /* set one entry, to_bytes, and assert the compressed bytes of an untouched member are byte for byte the ones of the input */ }
#[test] fn the_manifest_is_regenerated() { /* after set_entry the ./manifest.xml member of the output holds the new location */ }
#[test] fn recompress_writes_every_member_deflated() { /* to_bytes_recompressed(9): every member has method 8 and reads back equal */ }
#[test] fn an_empty_archive_writes_a_valid_zip_with_one_member() { /* Archive::new().to_bytes() holds ./manifest.xml only */ }
#[test] fn python_zipfile_reads_what_the_core_writes() { /* write the four multi-entry fixtures to target/tmp, run scripts/read_back.py, assert exit code 0 */ }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pymetadata-core --locked --test write`
Expected: FAIL, `zip::Writer` not found.

- [ ] **Step 3: Write `scripts/read_back.py`**

It takes paths of written archives, opens each with `zipfile.ZipFile`, calls `testzip()`, asserts it is `None`, reads every member, and compares the manifest entries with `pymetadata.omex.Omex.from_omex`. A mismatch exits nonzero with the member named.

- [ ] **Step 4: Implement the writer and the two `Archive` methods**

`Writer` collects members into a buffer: local header, data, then the central directory and the end-of-central-directory record, with zip64 records when a size or the count exceeds the 32-bit fields. `add_raw` copies the compressed bytes with their CRC, sizes and method. `to_bytes` writes `./manifest.xml` from `self.manifest().to_xml()` deflated, every `Replaced` entry deflated at level 6, and every untouched member with `add_raw`. `to_bytes_recompressed` inflates everything and deflates at `level`. Both write the members in the manifest order and skip the location `.`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p pymetadata-core --locked`
Expected: PASS.

- [ ] **Step 6: Run every gate and commit**

```bash
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked && RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
git add -A
git commit -m "feat: write an archive by copying untouched entries as blobs"
```

---

### Task 7: The JSON wire types

**Files:**
- Create: `crates/core/src/wire.rs`
- Modify: `crates/core/src/lib.rs`
- Test: `crates/core/src/wire.rs` (unit tests)

**Interfaces:**
- Consumes: `Entry`, `Manifest` (Task 2), `OmexError` (Task 1).
- Produces: `wire::ManifestResponse { entries: Vec<Entry> }`, `wire::SetEntryRequest { location: String, format: Option<String>, master: bool, len: usize }`, `wire::ErrorResponse { kind: &'static str, message: String }`, `wire::kind(&OmexError) -> &'static str` returning `malformed`, `refused`, `unsupported` or `internal`.

- [ ] **Step 1: Write the failing tests** in `crates/core/src/wire.rs`

```rust
#[test] fn manifest_response_serializes_entries_in_order() { /* three entries, serde_json keeps the order, keys are location, format, master */ }
#[test] fn set_entry_request_parses_without_a_format() { /* format is None and master defaults to false */ }
#[test] fn error_kinds_are_stable() { /* each OmexError variant maps to its documented kind string */ }
```

- [ ] **Step 2: Run the tests to verify they fail, implement, and verify they pass**

Run: `cargo test -p pymetadata-core --locked wire`
Expected: FAIL, then PASS after `wire.rs` exists.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: the JSON wire types of the hosts"
```

---

### Task 8: The python binding

**Files:**
- Create: `crates/python/Cargo.toml`, `crates/python/src/lib.rs`, `crates/python/src/archive.rs`, `crates/python/src/errors.rs`
- Create: `pyproject.toml`, `python/pymetadata_core/__init__.py`, `python/pymetadata_core/_core.pyi`, `python/pymetadata_core/py.typed`, `python/README.md`, `ruff.toml`
- Test: `python/tests/test_archive.py`, `python/tests/test_roundtrip.py`, `python/tests/test_errors.py`

**Interfaces:**
- Consumes: the whole public surface of `pymetadata-core` (Tasks 1 to 7).
- Produces: the module `pymetadata_core` with `Archive` (`from_bytes(data, password=None)`, `entries` property of `list[tuple[str, str, bool]]`, `manifest_xml()`, `entry_bytes(location)`, `set_entry(location, format, master, data)`, `remove_entry(location)`, `to_bytes(recompress=False, level=9)`), `normalize_location(location)`, `guess_format(suffix, head=b"")`, `lookup_format(format_key)`, `OmexError(ValueError)`, `__version__`.

- [ ] **Step 1: Write the failing python tests**

```python
def test_opens_every_fixture():  # the six archives: entries is not empty, "." and "./manifest.xml" are present
def test_entry_bytes_matches_zipfile():  # per archive and member, equal to zipfile.ZipFile(...).read(name)
def test_set_entry_then_to_bytes_is_readable_by_zipfile():  # testzip() is None, the new member is there
def test_roundtrip_through_pymetadata():  # pymetadata.omex.Omex.from_omex reads the written archive and sees the same manifest
def test_normalize_location_matches_pymetadata():  # the same inputs as the rust tests, against pymetadata.omex.normalize_location
def test_malformed_bytes_raise_omex_error():  # b"not a zip" raises OmexError, which is a ValueError
def test_unknown_location_raises_omex_error():
def test_wrong_password_raises_omex_error():
```

- [ ] **Step 2: Run them to verify they fail**

Run: `uvx maturin build --release --locked -o dist && pip install --force-reinstall dist/*.whl && pytest python/tests -q`
Expected: FAIL, no module named `pymetadata_core`.

- [ ] **Step 3: Write the crate and the packaging**

`crates/python/Cargo.toml`: package `pymetadata-python`, `[lib] name = "pymetadata_core"`, `crate-type = ["cdylib", "rlib"]`, dependency on `pymetadata-core` by path and version, workspace `pyo3` with `abi3-py311` (the floor of `pymetadata`), the optional feature `extension-module = ["pyo3/extension-module"]`. Root `pyproject.toml`: `build-backend = "maturin"`, `requires = ["maturin>=1.16,<2"]`, distribution name `pymetadata-core`, `requires-python = ">=3.11"`, `dynamic = ["version"]`, `license-files = ["LICENSE"]`, classifier `Typing :: Typed`, `[tool.maturin] manifest-path = "crates/python/Cargo.toml"`, `python-source = "python"`, `module-name = "pymetadata_core._core"`, `features = ["extension-module"]`. `python/pymetadata_core/__init__.py` re-exports the names of the extension and `__version__`; `_core.pyi` carries the signatures above with full annotations and google-style docstrings; `py.typed` is empty.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `uvx maturin build --release --locked -o dist && pip install --force-reinstall dist/*.whl && pytest python/tests -q`
Expected: PASS.

- [ ] **Step 5: Run the python gates and commit**

```bash
uvx ruff@0.17.5 check && uvx ruff@0.17.5 format --check && cargo clippy -p pymetadata-python --locked --all-targets -- -D warnings
git add -A
git commit -m "feat: the python binding of the archive"
```

---

### Task 9: The wasm host

**Files:**
- Create: `crates/wasm/Cargo.toml`, `crates/wasm/src/lib.rs`, `crates/wasm/src/buffers.rs`, `crates/wasm/src/calls.rs`, `crates/wasm/src/panic.rs`
- Test: `crates/wasm/tests/imports.rs`, `crates/wasm/tests/calls.rs`

**Interfaces:**
- Consumes: `Archive`, `wire` (Tasks 5 to 7).
- Produces: the exports `omex_alloc`, `omex_output_ptr`, `omex_output_len`, `omex_open`, `omex_manifest`, `omex_entry`, `omex_set_entry`, `omex_remove_entry`, `omex_write`, `omex_about`, each `extern "C"` returning `i32`, 0 for an answer; and the pure functions behind them, which the native tests call.

- [ ] **Step 1: Write the failing tests**

`crates/wasm/tests/imports.rs`: the sibling test, ported, asserting the built module at `target/wasm32-unknown-unknown/release/pymetadata_wasm.wasm` has no import section.
`crates/wasm/tests/calls.rs`:

```rust
#[test] fn open_then_manifest_returns_the_entries_as_json() { /* a fixture archive: the JSON parses into wire::ManifestResponse with the expected locations */ }
#[test] fn entry_returns_the_inflated_bytes() { }
#[test] fn set_entry_then_write_produces_a_readable_archive() { /* Archive::from_bytes of the output succeeds and holds the new entry */ }
#[test] fn a_malformed_archive_answers_with_the_malformed_kind() { /* the output buffer holds an ErrorResponse with kind "malformed" */ }
#[test] fn about_names_the_version_and_the_backend() { /* version equals env!("CARGO_PKG_VERSION"), backend is "zlib-rs" */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo build -p pymetadata-wasm --release --target wasm32-unknown-unknown && cargo test -p pymetadata-wasm --locked`
Expected: FAIL.

- [ ] **Step 3: Implement the host**

`buffers.rs` holds the input and output buffers and the archive as module state, reached through raw pointers as the spike's `wasm.rs` does. `calls.rs` holds one pure function per export, taking and returning owned data, so the native tests exercise them. `lib.rs` holds the `#[unsafe(no_mangle)] extern "C"` wrappers, which only move bytes and map an `OmexError` to its kind and code. `panic.rs` installs a panic hook writing the message into the output buffer before the trap.

- [ ] **Step 4: Run the tests to verify they pass, then check both wasm targets**

```bash
cargo build -p pymetadata-wasm --release --target wasm32-unknown-unknown
cargo test -p pymetadata-wasm --locked
cargo check --locked --workspace --exclude pymetadata-python --target wasm32-unknown-unknown
cargo check --locked --workspace --exclude pymetadata-python --target wasm32-wasip1
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: the WebAssembly host of the archive"
```

---

### Task 10: The JavaScript package

**Files:**
- Create: `js/package.json`, `js/loader.mjs`, `js/build.mjs`, `js/prepack.mjs`, `js/version.mjs`, `js/README.md`, `js/.gitignore`
- Test: `js/test/loader.test.mjs`, `js/test/package.test.mjs`, `js/test/fixtures/` (one archive, copied by `build.mjs`)

**Interfaces:**
- Consumes: the wasm exports (Task 9).
- Produces: `createRuntime({ module })` returning `{ open(bytes), manifest(), entry(location), setEntry(location, format, master, bytes), removeEntry(location), write({ recompress }), about() }`, with no Node API and no dependency, plus the npm package `@matthiaskoenig/pymetadata-core`.

- [ ] **Step 1: Write the failing node tests**

```javascript
test('open and manifest list the entries of a fixture archive')
test('entry returns the inflated bytes of a member')
test('setEntry then write produces an archive the runtime reads back')
test('a malformed archive rejects with the malformed kind')
test('package.json version equals the crate version')   // package.test.mjs
test('the loader touches no Node API')                   // source scan for node: and require(
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd js && node --test test/*.test.mjs`
Expected: FAIL.

- [ ] **Step 3: Implement `build.mjs`, `loader.mjs` and the package files**

`build.mjs` runs `cargo build -p pymetadata-wasm --release --target wasm32-unknown-unknown`, copies the module to `js/runtime/pymetadata-core.wasm`, prints the byte and gzipped sizes, and with `--check` fails when the module is missing or older than the newest mtime under `crates/` or `Cargo.lock`. `loader.mjs` instantiates with `{}`, wraps the exports, grows the input buffer through `omex_alloc`, and throws an `Error` carrying `kind` and `message` from the output buffer for a nonzero code. `package.json`: name `@matthiaskoenig/pymetadata-core`, version `0.1.0`, `type: module`, `engines.node >= 20`, no dependencies, exports `"."` to `./loader.mjs` and the wasm path, files the loader plus `runtime/pymetadata-core.wasm`, `LICENSE` and `README.md`, scripts `build`, `check`, `test`, `prepack`, `publishConfig` with `access: public` and `provenance: true`. `prepack.mjs` refuses when the package version differs from the crate version.

- [ ] **Step 4: Run the tests to verify they pass, then commit**

```bash
cd js && node build.mjs && node --test test/*.test.mjs && npm pack --dry-run
git add -A
git commit -m "feat: the JavaScript loader of the wasm module"
```

---

### Task 11: Benchmarks

**Files:**
- Create: `crates/core/examples/omex_timing.rs`, `scripts/python_timing.py`, `BENCHMARKS.md`, `bench/RESULTS.md`, `js/bench.mjs`, `js/bench.html`
- Test: none (a benchmark is not a gate)

**Interfaces:**
- Consumes: `Archive` (Tasks 5, 6), the loader (Task 10).
- Produces: `cargo run --release --example omex_timing`, `python scripts/python_timing.py`, `node js/bench.mjs`.

- [ ] **Step 1: Write `omex_timing.rs`**

The protocol of `spike/python_timing.py`: 3 warm-up calls, 3 rounds of 20 timed calls, the number is the median of the round medians, one checksum per cell. The cells are `manifest_only`, `read_all`, `modify_write` and `write_recompress` over the six fixtures. It prints a markdown table and `--csv` writes the raw numbers.

- [ ] **Step 2: Write `scripts/python_timing.py`**

The same protocol against the installed `pymetadata`, with the cells `from_omex`, `to_omex`, `manifest_only` and `read_all`, and `gc.collect()` before each timed call.

- [ ] **Step 3: Run both and write `BENCHMARKS.md`**

Run: `cargo run --release --example omex_timing` and `cd /home/mkoenig/git/pymetadata && uv run python ../pymetadata-core/scripts/python_timing.py`. `BENCHMARKS.md` carries "What is inside each timing", "The machine", "The protocol", the comparison table and the sentence that neither table is a gate.

- [ ] **Step 4: Write `js/bench.mjs`, `js/bench.html` and `bench/RESULTS.md`**

The node and Chrome numbers for `open`, `manifest`, `entry` and `write` on the smallest and the largest fixture, between `<!-- numbers:start -->` markers.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs: the benchmark harnesses and their first numbers"
```

---

### Task 12: Continuous integration, release and documentation

**Files:**
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `.github/dependabot.yml`, `.github/rulesets/` (copied from `pymetadata` and adapted), `release-notes/0.1.0.md`, `docs/release.md`, `README.md` (the Checks section)
- Test: the workflows themselves, run on the pull request

**Interfaces:**
- Consumes: every gate command of Tasks 1 to 11.

- [ ] **Step 1: Write `ci.yml` with the jobs `checks`, `wasm`, `js`, `python` and `ruff`**

The commands are the ones in the spec's "Repository conventions", actions pinned to exact versions as in `/home/mkoenig/git/pkpdutils-core/.github/workflows/ci.yml`, `cancel-in-progress: true`.

- [ ] **Step 2: Write `release.yml`**

The wheel matrix through `PyO3/maturin-action` with an install and pytest smoke test per wheel, the sdist, `publish` to PyPI with trusted publishing from the `pypi` environment on a `v*` tag which reads `release-notes/0.1.0.md` first, then `npm` with provenance, then `github_release` with the notes as its body. A pull request runs it as a dry run and publishes nothing.

- [ ] **Step 3: Write `dependabot.yml`, `docs/release.md` and `release-notes/0.1.0.md`**

Dependabot monthly, github-actions grouped, cargo with `versioning-strategy: lockfile-only`, npm and pip. `docs/release.md` is the release flow: branch `release/x.y.z` off `develop`, the notes, the version bump of the workspace, the path-dependency pins and `js/package.json`, `cargo metadata` to refresh the lock, the gates, the pull request, then the tag on `develop`.

- [ ] **Step 4: Verify every gate locally, then open the pull request**

```bash
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo check --locked --workspace --exclude pymetadata-python --target wasm32-unknown-unknown
cd js && node build.mjs --check && node --test test/*.test.mjs
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "ci: the checks and the release workflow"
```

---

## After this plan

Two follow-ups, each its own plan:

1. The hard swap in `pymetadata`: `pymetadata.omex` over `pymetadata_core`, the dependency and the wheel matrix, with `tests/test_omex.py` unchanged as the gate. It needs `pymetadata-core` 0.1.0 on PyPI.
2. The lookup milestone of issue #99: the identifiers.org registry and the annotation parsing, with its own design document.
