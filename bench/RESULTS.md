# Browser and node numbers

The wasm module through `js/loader.mjs`, in node and in headless Chrome, measured with
`js/bench.mjs` and `js/bench.html`. The rust and python numbers are in `BENCHMARKS.md`, which
also holds the machine and the shared-machine caveat. Neither table is a gate: no test runs the
harness and no number is asserted. The machine was shared; measure again on an idle machine
before quoting a number of them in a paper.

## What is inside each timing

Every cell starts from the bytes of the archive in a `Uint8Array`, so the copy into the linear
memory is in every cell which opens.

- `open`: `runtime.open(bytes)`. The module copies the bytes in, reads the central directory and
  parses the manifest.
- `manifest`: `runtime.manifest()` on an open archive, the manifest as JSON parsed into objects.
- `open_manifest`: `open` and `manifest`, the partner of `manifest_only` in `BENCHMARKS.md`.
- `entry`: `open`, then `entry` of the largest entry (7 MB inflated for `iCGB21FR.omex`). It is
  `open` and not `entry` alone, because the module keeps an inflated entry and a second call would
  time the cache; subtract `open` to read the inflate.
- `write`: `open`, `setEntry` of 64 kB for the first entry which has bytes, and `write()`. The
  partner of `modify_write`: the entries which did not change are copied as blobs.
- `write_recompress`: `open` and `write({ recompress: true })`, which deflates at level 9.

## The protocol

That of `BENCHMARKS.md`: 3 warm-up samples, then 3 rounds of 20 samples, and the number is the
median of the round medians, the fastest of 5 runs in node and of 3 runs in Chrome (`bench/runs/`).
One difference: a sample is a batch of calls (the `batch` of `bench/runs/*.json`, doubled until a
batch takes 2 ms) and the number is the time of a batch divided by the calls in it. A call of
`open` takes microseconds and `performance.now()` of a page which is not cross-origin isolated is
clamped to 0.1 ms in Chrome, so a single call cannot be timed there. The checksum is the result of
the last call and is the same in node and in Chrome for every cell (entries, bytes, size written).
The Chrome numbers are quantized for that reason: a cell is a multiple of 0.1 ms divided by its
batch (0.025 ms for `write` of the 1.4 MB archive, a batch of 4; 0.1 ms for a batch of 1), so
digits below that are not information, and the cells of 0.01 ms and below are the least certain.
The Chrome load was not recorded.

```bash
node js/build.mjs
node js/bench.mjs --json bench/runs/node-1.json
python3 -m http.server 8000   # from the repository root, then open /js/bench.html?auto
python3 scripts/bench_table.py js
```

## Numbers

Node v24.21.0 and HeadlessChrome 157.0.8103.78 (V8 in both), 12th Gen Intel Core i7-12700,
Linux 7.0.0-43-generic, measured 2026-11-23, module of 394 kB (168 kB gzipped). The smallest
(`CompModels.omex`, 6153 bytes, shown as 7 kB) and the largest (`iCGB21FR.omex`, 1.4 MB) archive.

<!-- numbers:start -->
| archive | size | op | node ms | chrome ms |
|---|---|---|---|---|
| CompModels.omex | 7 kB | open | 0.0129 | 0.0281 |
| CompModels.omex | 7 kB | manifest | 0.0025 | 0.0037 |
| CompModels.omex | 7 kB | open_manifest | 0.0155 | 0.033 |
| CompModels.omex | 7 kB | entry | 0.0250 | 0.052 |
| CompModels.omex | 7 kB | write | 0.1313 | 0.231 |
| CompModels.omex | 7 kB | write_recompress | 0.3557 | 0.59 |
| iCGB21FR.omex | 1.4 MB | open | 0.1693 | 0.234 |
| iCGB21FR.omex | 1.4 MB | manifest | 0.0072 | 0.0113 |
| iCGB21FR.omex | 1.4 MB | open_manifest | 0.1820 | 0.25 |
| iCGB21FR.omex | 1.4 MB | entry | 7.6637 | 10.2 |
| iCGB21FR.omex | 1.4 MB | write | 0.5071 | 0.95 |
| iCGB21FR.omex | 1.4 MB | write_recompress | 156.7229 | 241.6 |
<!-- numbers:end -->

Read against the native numbers of `BENCHMARKS.md` (`iCGB21FR.omex`, same work, the module copies
the staged input in as well):

| cell | native ms | node ms | chrome ms | node / native | chrome / native |
|---|---|---|---|---|---|
| manifest only (`manifest_only` / `open_manifest`) | 0.078 | 0.182 | 0.250 | 2.3x | 3.2x |
| modify, write (`modify_write` / `write`) | 0.193 | 0.507 | 0.95 | 2.6x | 4.9x |
| recompress at level 9 (`write_recompress`) | 125.9 | 156.7 | 241.6 | 1.2x | 1.9x |

The wasm module is 1.2x to 2.6x slower than native in node and 1.9x to 4.9x in Chrome, and the
short cells lose the most, which is probably the copies of the input and the call boundary, which
this run did not measure. Node and Chrome run the same engine, and this run does not say why
Chrome is slower. The cells which copy blobs (`write`, 0.51 ms in node) stay far below the ones
which deflate.
