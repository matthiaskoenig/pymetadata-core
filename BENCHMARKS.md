# Benchmarks

What a lazy Rust core is worth against `pymetadata` 0.8.3 on COMBINE archives, measured with the
harnesses in this repository. It follows `spike/RESULTS.md` and is comparable with it: the same
four archives, the same protocol. The browser and node numbers of the wasm module are in
`bench/RESULTS.md`.

Neither table is a gate. Nothing in continuous integration, `cargo test`, `pytest` or `npm test`
times anything or asserts a number; each harness is run by hand.

## What is inside each timing

Rust (`crates/core/examples/omex_timing.rs`) starts every cell from the bytes of the archive in
memory and does no file I/O. Every cell but `manifest_owned` goes through `Archive::from_bytes`,
which takes `&[u8]`, so it copies its input (the zip reader owns a `Vec`), reads the central
directory, inflates and parses `manifest.xml`, and inflates nothing else. The copy is made inside
the call, and in this harness it is a copy of the fixture the benchmark keeps, so it is a cost of
the call as the API is shaped today; a caller who already owns a `Vec` would not pay it (see
the follow-up in `docs/design/omex-core.md`).

- `manifest_only`: `from_bytes` and the entries of the manifest, input copied. This is the
  request a page makes most. It also adds the members the manifest does not list, so it counts
  more entries than python does.
- `manifest_owned`: the same request with the input already owned. Each call gets its own `Vec`
  made before the clock starts, `Reader::new` takes it, the manifest member is inflated and
  parsed. It is not `from_bytes` minus the copy: it also skips the pass over the members which
  adds the unlisted ones, so it is the floor of the request and not an API call.
- `read_all`: `from_bytes` and an inflate of every entry which has bytes. The manifest of
  `iCGB21FR.omex` lists entries the zip does not hold; they have no bytes and are skipped.
- `modify_write`: `from_bytes`, `set_entry` with 64 kB for the first entry which has bytes, and
  `to_bytes`. Every entry which did not change is copied as the compressed blob it is; only the
  64 kB are deflated.
- `classify_heads`: `from_bytes` and `Archive::head` for the first 256 bytes of every entry
  which has bytes. It is `read_all` with the head in place of the whole member, so the two are
  the same walk over the same entries, inflating 256 bytes of each or all of each. Added in
  0.3.0.
- `probe`: `zip::probe::declares_manifest_in` over the same bytes, which parses the records at
  the end of the container and walks its central directory for the name of the manifest. It
  inflates nothing and opens no archive, and it returns as soon as it finds the name, which in
  every fixture of the corpus is within the first members. Added in 0.3.0.
- `write_copy`: `from_bytes` and `to_bytes`, nothing changed. The raw copy alone.
- `write_recompress`, `write_recompress_l6`: `from_bytes` and `to_bytes_recompressed` at level 9
  and at level 6. Every member is inflated and deflated again.
- `input_copy`: the copy of the input `from_bytes` makes, and nothing else, 0.047 ms for the
  1.4 MB archive.

Python (`scripts/python_timing.py`) runs against the installed `pymetadata` 0.8.3 from PyPI,
the one `uv.lock` pins, in this repository's own environment (`uv run --no-sync`). It needs
nothing the wheel lacks.

- `from_omex`, `to_omex`, `load_modify_save` are the public API. `from_omex` extracts the whole
  archive into a temporary directory and builds the pydantic manifest. `to_omex` writes to a path
  through a temporary file, deflating every entry at level 9, its default, from the files of
  that directory. `load_modify_save` is `from_omex`, `add_entry` with the same 64 kB and the same
  entry as the Rust cell, and `to_omex`. These touch the file system, with a warm page cache.
- `manifest_only`: `zipfile` on the bytes, `ElementTree`, and the pydantic `Manifest`.
- `read_all`: `zipfile` inflating every entry but the manifest, from the bytes.
- `zip_write_recompress`, `zip_write_recompress_l6`: `zipfile` reading the archive and writing it
  again in memory at level 9 and 6. This is `to_omex` without the directory and the path, and the
  like for like partner of Rust `write_recompress`. Level 6 is the default of zlib and of
  `zipfile`, and what the spike measured.

## The machine

Measured on 2026-11-23: 12th Gen Intel Core i7-12700 (20 threads, 62 GB), Linux 7.0.0-43-generic
x86_64, rustc 1.100.0 with `--release` (the workspace profile: `lto`, `codegen-units = 1`),
python 3.13.18 (the spike used 3.14.10) with pymetadata 0.8.3, node v24.21.0, Chrome 157.0.8103.78
(headless). The deflate backend is zlib-rs.

The machine was shared. A simulation job of another project kept the load average between 2 and
22 while this ran, and a run which overlaps one of its bursts is slower: a first set of runs had
`write_recompress` of `iCGB21FR.omex` between 122 and 340 ms. The tables are not from an idle
machine. Measure again on an idle machine before quoting a number of them in a paper.

## The protocol

One call is timed with `perf_counter` or `Instant` around the cell alone. Then, exactly:

1. Warm-up: 3 calls, not recorded.
2. A round is 20 timed calls, and its value is the median of the 20.
3. A run is 3 rounds, and its value is the median of the 3 round values. That is the
   protocol of `spike/RESULTS.md`. A run is one line of a file in `bench/runs/`.
4. The harness is run 5 times (node 5, Chrome 3), one after the other, and the number in a table
   is the minimum of the run values of that cell, that is the fastest run, per cell and
   independently of the other cells. The minimum is the estimator least contaminated by
   interference, because load can only add time; it is also the most favourable one, so the
   slowest run is published next to it for the headline cells below. `scripts/bench_table.py`
   computes every table from `bench/runs/`.

The python side calls `gc.collect()` before each timed call and does not time it. Every cell
returns a checksum which is printed, and the harnesses keep the result of every call alive
(`black_box` in Rust, a list in python), so no call is removed as dead work. The checksums are the
bytes read, the entries counted or the size written, and they agree where the work is the same:
`read_all` gives the same number of bytes in Rust and python for all four archives (645716,
2219244, 24827, 11828301). They differ where it is not: the sizes of a written archive differ
with the deflate implementation, and `manifest_only` of Rust counts the two required entries and
the members the manifest does not list (24 for `iCGB21FR.omex`, 8 listed).

```bash
cargo run --release --locked --example omex_timing -- --csv bench/runs/rust-1.csv
uv run --no-sync python scripts/python_timing.py --csv bench/runs/python-1.csv
node js/bench.mjs --json bench/runs/node-1.json
python3 scripts/bench_table.py comparison     # the table below, from bench/runs/
```

What actually happened. Python and node ran 5 times in a loop with Rust, load average written
to `bench/runs/load.txt` around each iteration (load 1.9 to 10.2 at the start). The Rust numbers
of that loop were then discarded: `manifest_owned` was added after a review, and Rust was run 5
more times alone, at load 5 to 10 (the lines "rust rerun" of `load.txt`), while the python runs
had started at load 2 to 9. The published Rust numbers are the second set. A busier machine can
only make Rust slower, so that direction is against the core. The 3 Chrome runs were made
afterwards and their load was not recorded.

## Rust against python

The factor of a row is the python time divided by the rust time of the same row. Both columns are
the fastest of the runs. Every Rust cell is an API call: `manifest_only` is `Archive::from_bytes`
and so includes the copy of the input. The floor of the manifest request, which is not reachable
through any API, is not in this table but in its own table below.

| archive | size | cell | python ms | rust ms | factor |
|---|---|---|---|---|---|
| CompModels.omex | 7 kB | manifest_only (rust: from_bytes, input copied) | 0.143 | 0.007 | 19x |
| CompModels.omex | 7 kB | read_all | 0.145 | 0.032 | 4.5x |
| CompModels.omex | 7 kB | modify, write (python: from_omex, add_entry, to_omex) | 1.627 | 0.060 | 27x |
| CompModels.omex | 7 kB | write unchanged (python: to_omex) | 0.569 | 0.021 | 26x |
| CompModels.omex | 7 kB | recompress, level 9 (python: to_omex) | 0.569 | 0.269 | 2.1x |
| CompModels.omex | 7 kB | recompress, level 9 (python: zipfile, memory) | 0.530 | 0.269 | 2.0x |
| CompModels.omex | 7 kB | recompress, level 6 (python: zipfile, memory) | 0.519 | 0.210 | 2.5x |
| BIOMD0000000001.omex | 285 kB | manifest_only (rust: from_bytes, input copied) | 0.181 | 0.021 | 8.8x |
| BIOMD0000000001.omex | 285 kB | read_all | 1.471 | 1.167 | 1.3x |
| BIOMD0000000001.omex | 285 kB | modify, write (python: from_omex, add_entry, to_omex) | 9.602 | 0.182 | 53x |
| BIOMD0000000001.omex | 285 kB | write unchanged (python: to_omex) | 11.654 | 0.148 | 79x |
| BIOMD0000000001.omex | 285 kB | recompress, level 9 (python: to_omex) | 11.654 | 8.877 | 1.3x |
| BIOMD0000000001.omex | 285 kB | recompress, level 9 (python: zipfile, memory) | 12.396 | 8.877 | 1.4x |
| BIOMD0000000001.omex | 285 kB | recompress, level 6 (python: zipfile, memory) | 9.330 | 6.595 | 1.4x |
| CombineArchiveShowCase.omex | 1.1 MB | manifest_only (rust: from_bytes, input copied) | 0.278 | 0.064 | 4.3x |
| CombineArchiveShowCase.omex | 1.1 MB | read_all | 4.139 | 3.321 | 1.2x |
| CombineArchiveShowCase.omex | 1.1 MB | modify, write (python: from_omex, add_entry, to_omex) | 51.912 | 0.166 | 313x |
| CombineArchiveShowCase.omex | 1.1 MB | write unchanged (python: to_omex) | 45.296 | 0.127 | 356x |
| CombineArchiveShowCase.omex | 1.1 MB | recompress, level 9 (python: to_omex) | 45.296 | 32.954 | 1.4x |
| CombineArchiveShowCase.omex | 1.1 MB | recompress, level 9 (python: zipfile, memory) | 48.257 | 32.954 | 1.5x |
| CombineArchiveShowCase.omex | 1.1 MB | recompress, level 6 (python: zipfile, memory) | 34.711 | 25.301 | 1.4x |
| iCGB21FR.omex | 1.4 MB | manifest_only (rust: from_bytes, input copied) | 0.197 | 0.078 | 2.5x |
| iCGB21FR.omex | 1.4 MB | read_all | 18.594 | 10.281 | 1.8x |
| iCGB21FR.omex | 1.4 MB | modify, write (python: from_omex, add_entry, to_omex) | 251.372 | 0.193 | 1304x |
| iCGB21FR.omex | 1.4 MB | write unchanged (python: to_omex) | 230.333 | 0.151 | 1520x |
| iCGB21FR.omex | 1.4 MB | recompress, level 9 (python: to_omex) | 230.333 | 125.894 | 1.8x |
| iCGB21FR.omex | 1.4 MB | recompress, level 9 (python: zipfile, memory) | 246.131 | 125.894 | 2.0x |
| iCGB21FR.omex | 1.4 MB | recompress, level 6 (python: zipfile, memory) | 122.988 | 76.178 | 1.6x |

Which cells are comparable.

- `read_all` and the `recompress` rows are the same work on both sides: inflate, or inflate
  and deflate, every entry, from bytes in memory (the `to_omex` row also writes a file). They are
  the honest rows. The core is 1.2x to 4.5x on reading and 1.3x to 2.5x on rewriting, which is
  zlib-rs against the zlib of python, and python's `zipfile` is not slow at what it does.
- `manifest_only` is nearly the same work: python reads the manifest member out of a
  `ZipFile(BytesIO(data))`, which shares the buffer and copies nothing, and builds the pydantic
  manifest. The rust row is 2.5x to 19x, with the copy. The floor table gives 14x to 26x, which no
  API reaches, and the 1.4 MB archive is where the copy matters most: 0.047 ms of 0.078. The rest
  of the 0.066 ms gap between `manifest_only` and `manifest_owned` (0.019 ms) is the pass which
  adds the unlisted members. `manifest_owned` is a floor and not an API call.
- `modify, write` and `write unchanged` are not the same work, and that is the result. The core
  copies the entries it was not asked to change; python re-deflates every one, because `to_omex`
  has no other way to write an entry. The factor (27x to 1304x for modify) is the price of the
  missing raw copy, and it grows with the archive, as the work of python does. Python also writes
  a file and reads the files of a temporary directory, which the core does not. Nothing in the
  factor says the core deflates faster: that is the `recompress` rows.
- `from_omex` has no partner. It extracts to disk, which the core never does, so no factor is
  published for it. Its time is below, with the copy of the input of Rust for scale.

| archive | python from_omex ms | rust input_copy ms |
|---|---|---|
| CompModels.omex | 0.817 | 0.0001 |
| BIOMD0000000001.omex | 2.653 | 0.0045 |
| CombineArchiveShowCase.omex | 6.543 | 0.0346 |
| iCGB21FR.omex | 20.294 | 0.0473 |

The floor of the manifest request, `manifest_owned`: the input is already owned and the pass which
adds the members the manifest does not list is skipped. It is not an API call and no caller can
reach it today, so it is not a row of the table above and its ratio is not a speedup of the core.
The table below gives it for each archive.

| archive | python manifest_only ms | rust manifest_owned ms | ratio to the floor |
|---|---|---|---|
| CompModels.omex | 0.143 | 0.005 | 26x |
| BIOMD0000000001.omex | 0.181 | 0.011 | 16x |
| CombineArchiveShowCase.omex | 0.278 | 0.020 | 14x |
| iCGB21FR.omex | 0.197 | 0.012 | 17x |

The headline cells on `iCGB21FR.omex` (1.4 MB), over the runs:

| cell on iCGB21FR.omex | runs | fastest ms | slowest ms |
|---|---|---|---|
| rust manifest_only | 5 | 0.078 | 0.090 |
| rust manifest_owned | 5 | 0.012 | 0.013 |
| rust modify_write | 5 | 0.193 | 0.246 |
| rust write_recompress | 5 | 125.894 | 144.559 |
| rust write_recompress_l6 | 5 | 76.178 | 127.887 |
| python manifest_only | 5 | 0.197 | 0.316 |
| python to_omex | 5 | 230.333 | 234.400 |
| python load_modify_save | 5 | 251.372 | 253.950 |
| python zip_write_recompress_l6 | 5 | 122.988 | 130.926 |

## The calls of 0.3.0

`classify_heads` and `probe` were added with the calls they time, so their numbers come from a
session of their own: five runs of the Rust harness on the machine below, written to
`bench/runs/rust-0.3.0-{1..5}.csv`, with the same protocol as the rest (3 warm-up calls, rounds
of 20 timed calls, the median of 3 rounds per run, the minimum of the 5 runs published). Both
columns of a row come from those same runs, so they are comparable with each other; they are
not comparable with the python columns above, which were measured in the earlier session.

The question each row answers is what the new call replaces: a host which classified a member
by its content had to inflate the member (`read_all`), and one which asked whether a file is an
archive had to open it (`manifest_only`).

| archive | size | read_all ms | classify_heads ms | factor |
|---|---|---|---|---|
| CompModels.omex | 7 kB | 0.0338 | 0.0162 | 2.1x |
| BIOMD0000000001.omex | 285 kB | 1.1962 | 0.0686 | 17x |
| CombineArchiveShowCase.omex | 1.1 MB | 3.3748 | 0.1523 | 22x |
| iCGB21FR.omex | 1.4 MB | 10.1016 | 0.1350 | 75x |

| archive | size | manifest_only ms | probe ms | factor |
|---|---|---|---|---|
| CompModels.omex | 7 kB | 0.0079 | 0.0007 | 11x |
| BIOMD0000000001.omex | 285 kB | 0.0229 | 0.0001 | 229x |
| CombineArchiveShowCase.omex | 1.1 MB | 0.0668 | 0.0012 | 56x |
| iCGB21FR.omex | 1.4 MB | 0.0801 | 0.0002 | 400x |

What these numbers are worth, and what they are not:

- The factor of `classify_heads` grows with the archive, which is the shape of the change: the
  head is bounded by 256 bytes per member while the inflate is not, so the bigger the members,
  the more of them is not read. The 2.1x of the 7 kB fixture is the floor, where the members
  are smaller than a few heads and the cost is the walk itself.
- The spread between the runs is wide for both cells, wider than for the older ones: the
  slowest of the five runs of `classify_heads` is 0.3460 ms for `iCGB21FR.omex` against the
  0.1350 ms published, and of `probe` 0.0004 ms against 0.0002 ms. The minimum is the
  favourable estimator the protocol prescribes; the slowest run still leaves the factor at 29x
  for `classify_heads` and 200x for `probe` on that archive.
- The `probe` numbers are tenths of a microsecond, which is the resolution the harness works
  at: read them as "this is not work", not as a measurement with three digits. It walks the
  central directory until it finds the manifest, which is the first member in each fixture of
  the corpus, so a container which declares it last would cost the whole directory.
- Neither cell does file I/O, so neither says what the python `has_manifest(path)` costs: that
  reads the tail of the file and the central directory from disk. Measured separately on
  `iCGB21FR.omex`, it is 0.005 ms against 0.434 ms for `Archive.from_bytes` over the same file,
  with the file never entering python memory.

## Against the spike

`spike/RESULTS.md` is the historical record and measured a narrower scope. Where the two differ,
this document supersedes it.

- Writing. The spike measured 233.9 ms for `to_omex` of `iCGB21FR.omex` and 0.12 ms for the raw
  copy write (about 1950x). This measures 230.3 ms and 0.151 ms (1520x). The raw copy here
  includes opening the archive, the copy of the input and writing the manifest, which the
  spike's `modify_rawcopy` did not. The orders of magnitude are the same.
- The manifest. The spike measured 0.155 ms against 0.014 ms (11x) for `iCGB21FR.omex`. With
  the input owned and without the unlisted-members pass (a floor, not an API call) this measures
  0.197 ms against 0.012 ms (17x). Through `from_bytes` it is 0.078 ms (2.5x), because the call
  copies its input (0.047 ms) and adds the unlisted members (0.019 ms), which the spike's reader
  did not. Which of the two a user sees depends on the API: today the second.
- Reading every entry: 1.8x for `iCGB21FR.omex` (the spike: 2.0x), 1.2x to 4.5x over the
  archives (the spike: 1.3x to 6.6x).
- Recompressing. At level 6, the level of the spike, 76 ms against 123 ms (1.6x) for
  `iCGB21FR.omex`, against the spike's 71 ms and 126 ms. At level 9, which `to_omex` uses, the
  core takes 126 ms and `zipfile` 246 ms (2.0x), or 1.8x against `to_omex` at 230 ms: python at
  level 9 is twice as slow as at level 6, and the core 1.7x, so the factor depends on the level.

## The write which does not hold the archive

`Archive::write_into`, and `Archive.to_path` in python, write into a sink instead of building
the archive in a `Vec` (#12). This is a measurement of memory rather than of time, so it does
not follow the protocol above: one archive, one run, the peak RSS of the process
(`ru_maxrss`) and the wall clock of the call, python 3.14 on the machine below.

The archive is 268.5 MB with one incompressible member, opened with `Archive.from_bytes`, which
leaves the process at 525.7 MB: the `bytes` of the file and the copy the reader owns.

| write | peak RSS it adds | time |
|---|---|---|
| `to_path` | 0.0 MB | 46 ms |
| `to_bytes` plus `Path.write_bytes` | 256.5 MB | 163 ms |

The two files are byte for byte the same. The streamed write adds nothing measurable because
nothing of the archive is held: each member goes into the buffered file as it is produced, and
what stays is the central directory. It is also three times faster here, because the bytes are
copied once into the file instead of into a `Vec`, then into a python `bytes`, then into the
file.

What this does not change is the read: `from_bytes` takes a slice and copies it, so an opened
archive still holds its container, which is the 525.7 MB above. That is a separate cost and a
separate change.
