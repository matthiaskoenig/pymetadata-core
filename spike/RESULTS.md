# First spike: what a Rust OMEX core is worth

This package is throwaway. It exists to answer three questions with numbers
before the real crates are laid out, in the way the first spike of
`pkpdutils-core` did:

1. How much faster is a Rust OMEX reader and writer than `pymetadata.omex`?
2. Does the zip and XML stack build for `wasm32-unknown-unknown` with no
   imports, so a page instantiates the module with `{}`?
3. Which deflate backend: `zlib-rs` or `miniz_oxide`?

Neither table below is a gate. Nothing in continuous integration times
anything.

## The machine

Measured on 2026-11-23, 12th Gen Intel Core i7-12700 (20 threads), Linux
7.0.0-43-generic x86_64, rustc 1.100.0, `--release`, python 3.14.10 with
pymetadata 0.8.3, node v24.21.0.

## The protocol

The one of `pkpdutils-core`: 3 warm-up calls, then 3 rounds of 20 timed
calls, and the number is the median of the three round medians. The python
side (`python_timing.py`) repeats the protocol and calls `gc.collect()`
before each timed call. Both sides compute a checksum so a faster
implementation cannot be a wrong one. The archives are the six under
`tests/data/omex/` of pymetadata; the four below are the ones with more than
one entry.

## What is inside each timing

- `manifest_only`: read `manifest.xml` and parse it into entries. Nothing
  else is inflated. This is the request a frontend makes most.
- `read_all`: inflate every entry into memory.
- `extract_all`: inflate every entry and write it into a temporary
  directory, which is what `Omex.from_omex` does today.
- `modify_write`: replace one entry with 64 kB and write the archive again.
  Python re-deflates every entry. Rust (`modify_rawcopy`) copies the
  untouched entries as still-compressed blobs.
- `write_recompress`: write the archive again, deflating every entry. The
  comparable path, and what python has no alternative to.
- `pymetadata_from_omex` / `pymetadata_to_omex`: the public API of
  pymetadata 0.8.3, pydantic manifest included.

## Rust (zlib-rs) against python

| archive | size | entries | op | python ms | rust ms | factor |
|---|---|---|---|---|---|---|
| CompModels.omex | 6 kB | 6 | manifest_only | 0.118 | 0.007 | 17x |
| CompModels.omex | 6 kB | 6 | read_all | 0.186 | 0.028 | 6.6x |
| CompModels.omex | 6 kB | 6 | extract_all | 0.561 | 0.113 | 5.0x |
| CompModels.omex | 6 kB | 6 | modify + write | 0.550 | 0.037 | 15x |
| CompModels.omex | 6 kB | 6 | write_recompress | 0.533 | 0.208 | 2.6x |
| BIOMD0000000001.omex | 284 kB | 13 | manifest_only | 0.127 | 0.022 | 5.7x |
| BIOMD0000000001.omex | 284 kB | 13 | read_all | 1.535 | 1.038 | 1.5x |
| BIOMD0000000001.omex | 284 kB | 13 | extract_all | 2.286 | 1.323 | 1.7x |
| BIOMD0000000001.omex | 284 kB | 13 | modify + write | 9.439 | 0.158 | 60x |
| BIOMD0000000001.omex | 284 kB | 13 | write_recompress | 9.633 | 6.409 | 1.5x |
| CombineArchiveShowCase.omex | 1.1 MB | 22 | manifest_only | 0.175 | 0.018 | 9.8x |
| CombineArchiveShowCase.omex | 1.1 MB | 22 | read_all | 4.152 | 3.166 | 1.3x |
| CombineArchiveShowCase.omex | 1.1 MB | 22 | extract_all | 5.685 | 3.753 | 1.5x |
| CombineArchiveShowCase.omex | 1.1 MB | 22 | modify + write | 35.018 | 0.101 | 346x |
| CombineArchiveShowCase.omex | 1.1 MB | 22 | write_recompress | 35.100 | 25.176 | 1.4x |
| iCGB21FR.omex | 1.4 MB | 8 | manifest_only | 0.155 | 0.014 | 11x |
| iCGB21FR.omex | 1.4 MB | 8 | read_all | 19.047 | 9.488 | 2.0x |
| iCGB21FR.omex | 1.4 MB | 8 | extract_all | 18.340 | 12.122 | 1.5x |
| iCGB21FR.omex | 1.4 MB | 8 | modify + write | 125.985 | 0.120 | 1054x |
| iCGB21FR.omex | 1.4 MB | 8 | write_recompress | 126.133 | 71.318 | 1.8x |

Against the public API of pymetadata, which is what a caller actually pays:

| archive | `from_omex` ms | rust extract_all ms | factor | `to_omex` ms | rust modify_rawcopy ms | factor |
|---|---|---|---|---|---|---|
| CompModels.omex | 0.882 | 0.113 | 7.8x | 0.632 | 0.037 | 17x |
| BIOMD0000000001.omex | 2.949 | 1.323 | 2.2x | 11.910 | 0.158 | 75x |
| CombineArchiveShowCase.omex | 6.619 | 3.753 | 1.8x | 46.129 | 0.101 | 456x |
| iCGB21FR.omex | 19.267 | 12.122 | 1.6x | 233.903 | 0.120 | 1957x |

Three things to take from this.

- Inflate is not where the win is. Reading every entry is 1.3x to 2x, and
  `zipfile` plus zlib is not the bottleneck a frontend complains about.
- The manifest is 6x to 17x, and the absolute numbers (14 to 22 microseconds)
  matter for a page which lists the contents of an archive, or a tool which
  opens many archives to look at one attribute.
- Writing is where the orders of magnitude are. Python re-deflates the whole
  archive on every `to_omex`, so "load, change one file, save" costs 234 ms
  on a 1.4 MB archive. Copying the untouched entries as compressed blobs
  costs 0.12 ms. Even when every entry is genuinely re-deflated, Rust with
  zlib-rs is 1.4x to 2.6x.

Those numbers say the lazy design (approach 2) is the right one: the manifest
and the raw copy are both consequences of not inflating what nobody asked
for.

## The deflate backend

Same code, the two `flate2` backends, on the two large archives:

| archive | op | zlib-rs ms | miniz_oxide ms |
|---|---|---|---|
| CombineArchiveShowCase.omex | read_all | 3.166 | 3.781 |
| CombineArchiveShowCase.omex | write_recompress | 25.176 | 43.480 |
| iCGB21FR.omex | read_all | 9.488 | 13.671 |
| iCGB21FR.omex | write_recompress | 71.318 | 141.273 |

`miniz_oxide` is slower than python's zlib on `write_recompress`
(141 ms against 126 ms on `iCGB21FR.omex`), which would turn the one path
every writer takes into a regression. `zlib-rs` it is.

## wasm: zero imports

The first attempt used the `zip` crate (9.8.2, `default-features = false`,
`deflate-flate2`). It builds for `wasm32-unknown-unknown`, but the module
carries six imports:

```
__wbindgen_externref_xform__.__wbindgen_externref_statics_end
__wbindgen_placeholder__.__wbindgen_object_drop_ref
__wbindgen_placeholder__.__wbg_getRandomValues_8b6f1f663e6f188d
__wbindgen_placeholder__.__wbindgen_describe
__wbindgen_externref_xform__.__wbindgen_externref_table_set_null
__wbindgen_externref_xform__.__wbindgen_externref_table_grow
```

The cause is not a feature we can switch off. `zip` declares

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies.getrandom]
version = "0.4"
features = ["wasm_js"]
default-features = false
```

as a hard dependency, and `getrandom/wasm_js` pulls in `wasm-bindgen`. So the
`zip` crate costs the property that a page can instantiate the module with an
empty import object.

`wasm_check/` is the answer to that: the deflate, XML and CRC stack with no
zip crate, that is `flate2` (either backend), `roxmltree` and `crc32fast`.
Both backends build to a module with no imports, and a round trip through
`deflate` and `inflate` in node returns the input byte for byte:

| backend | module | imports | inflate 1.4 MB ms | deflate 1.4 MB ms |
|---|---|---|---|---|
| zlib-rs | 245 kB | 0 | 1.77 | 31.33 |
| miniz_oxide | 181 kB | 0 | 2.36 | 31.27 |
| `zip` 9.8.2 | 575 kB | 6 | - | - |

So the core reads and writes the zip container itself. That is no loss: the
format is a central directory, local headers and stored or deflated members,
the raw copy the numbers above depend on needs the raw members anyway, and
the sibling crates hand-roll their containers for the same reason.

Reproduce:

```bash
cargo run --release                                     # rust table, miniz_oxide
cargo run --release --no-default-features --features zlibrs
python python_timing.py                                 # python table
cd wasm_check
cargo build --release --target wasm32-unknown-unknown --no-default-features --features zlibrs
node node_test.mjs target/wasm32-unknown-unknown/release/spike2.wasm \
  ../../../pymetadata/tests/data/omex/iCGB21FR.omex
```
