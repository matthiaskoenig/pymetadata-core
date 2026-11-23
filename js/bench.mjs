// Timings of the wasm module through the loader, in node and (with bench.html) in a browser.
//
//   node js/bench.mjs [--json out.json]
//
// A benchmark is not a gate: nothing here is asserted, and `npm test` does not run it. The
// protocol is the one of crates/core/examples/omex_timing.rs and scripts/python_timing.py: 3
// warm-up samples, then 3 rounds of 20 samples, and the number is the median of the three round
// medians. One difference, which the timers force: a call of `open` takes microseconds and a
// browser clamps `performance.now()` to a few microseconds at best and 0.1 ms at worst, so a
// sample is a batch of `batch` calls, chosen once per cell so that a batch takes at least
// MIN_SAMPLE_MS, and the number is the time of a batch divided by `batch`. Every call returns a
// number which depends on its result; the one of the last call is printed as the checksum, and a
// call whose result is unused is not a call an engine can drop, because the module's calls have
// effects on its memory.
//
// The module is a fresh runtime per cell. The smallest and the largest archive of
// crates/core/tests/data/omex are measured.
import { createRuntime } from "./loader.mjs";

const WARMUP = 3;
const ROUNDS = 3;
const SAMPLES = 20;
const MIN_SAMPLE_MS = 2;
const PAYLOAD = new Uint8Array(64 * 1024).fill(120);

/** The median of an array of numbers. */
function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const n = sorted.length;
  return n % 2 === 1 ? sorted[(n - 1) / 2] : (sorted[n / 2 - 1] + sorted[n / 2]) / 2;
}

/**
 * Time `fn` by the protocol.
 *
 * @param {() => number} fn One call, returning a number which depends on its result.
 * @returns {{ms: number, rounds: number[], batch: number, checksum: number}} The checksum is
 *   the result of the last call, so it does not depend on the batch.
 */
function time(fn) {
  let checksum = 0;
  for (let i = 0; i < WARMUP; i++) checksum = fn();
  // the batch: double it until it takes MIN_SAMPLE_MS
  let batch = 1;
  for (;;) {
    const start = performance.now();
    for (let i = 0; i < batch; i++) checksum = fn();
    if (performance.now() - start >= MIN_SAMPLE_MS || batch >= 1 << 20) break;
    batch *= 2;
  }
  const rounds = [];
  for (let r = 0; r < ROUNDS; r++) {
    const samples = [];
    for (let s = 0; s < SAMPLES; s++) {
      const start = performance.now();
      for (let i = 0; i < batch; i++) checksum = fn();
      samples.push((performance.now() - start) / batch);
    }
    rounds.push(median(samples));
  }
  return { ms: median(rounds), rounds, batch, checksum };
}

/**
 * The cells of one archive.
 *
 * @param {WebAssembly.Module} module The compiled module.
 * @param {Uint8Array} bytes The archive.
 * @returns {Promise<Array<[string, () => number]>>}
 */
async function cells(module, bytes) {
  const runtime = await createRuntime({ module });
  runtime.open(bytes);
  const listed = runtime.manifest();
  // the first entry which has bytes is replaced, as the Rust and python cells do; the largest
  // is read, which is where the inflate time is
  let first = null;
  let largest = null;
  for (const entry of listed) {
    if (entry.location === "." || entry.location === "./manifest.xml") continue;
    let length;
    try {
      length = runtime.entry(entry.location).length;
    } catch (error) {
      if (error.kind === "not_found") continue;
      throw error;
    }
    first ??= entry;
    if (largest === null || length > largest.length) largest = { ...entry, length };
  }
  return [
    ["open", () => (runtime.open(bytes), 1)],
    ["manifest", () => runtime.manifest().length],
    ["open_manifest", () => (runtime.open(bytes), runtime.manifest().length)],
    ["entry", () => (runtime.open(bytes), runtime.entry(largest.location).length)],
    [
      "write",
      () => {
        runtime.open(bytes);
        runtime.setEntry(first.location, first.format, first.master, PAYLOAD);
        return runtime.write().length;
      },
    ],
    [
      "write_recompress",
      () => (runtime.open(bytes), runtime.write({ recompress: true }).length),
    ],
  ];
}

/**
 * Measure the smallest and the largest archive.
 *
 * @param {WebAssembly.Module} module The compiled module.
 * @param {Map<string, Uint8Array>} archives The archives by name.
 * @returns {Promise<Array<object>>} One row per archive and cell.
 */
export async function runBenchmarks(module, archives) {
  const bySize = [...archives].sort((a, b) => a[1].length - b[1].length);
  const picked = [bySize[0], bySize[bySize.length - 1]];
  const rows = [];
  for (const [name, bytes] of picked) {
    for (const [op, fn] of await cells(module, bytes)) {
      rows.push({ archive: name, size: bytes.length, op, ...time(fn) });
    }
  }
  return rows;
}

/**
 * A markdown table of rows.
 *
 * @param {Array<object>} rows The rows of {@link runBenchmarks}.
 * @returns {string}
 */
export function table(rows) {
  const size = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.ceil(n / 1e3)} kB`);
  const lines = ["| archive | size | op | ms | batch | checksum |", "|---|---|---|---|---|---|"];
  for (const r of rows) {
    lines.push(
      `| ${r.archive} | ${size(r.size)} | ${r.op} | ${r.ms.toFixed(4)} | ${r.batch} | ${r.checksum} |`,
    );
  }
  return lines.join("\n");
}

// the command line, which a browser never runs; its imports are dynamic for the same reason
if (typeof process !== "undefined" && process.versions?.node) {
  const { fileURLToPath } = await import("node:url");
  if (process.argv[1] === fileURLToPath(import.meta.url)) {
    const { readFileSync, readdirSync, writeFileSync } = await import("node:fs");
    const { cpus } = await import("node:os");
    const { dirname, join } = await import("node:path");
    const here = dirname(fileURLToPath(import.meta.url));
    const corpus = join(here, "../crates/core/tests/data/omex");
    const archives = new Map(
      readdirSync(corpus)
        .filter((name) => name.endsWith(".omex"))
        .map((name) => [name, new Uint8Array(readFileSync(join(corpus, name)))]),
    );
    const wasm = readFileSync(join(here, "runtime/pymetadata-core.wasm"));
    const rows = await runBenchmarks(await WebAssembly.compile(wasm), archives);
    console.log(table(rows));
    console.log(`\nnode ${process.version}, ${cpus()[0].model}, ${process.platform} ${process.arch}`);
    const json = process.argv.indexOf("--json");
    if (json > 0) writeFileSync(process.argv[json + 1], JSON.stringify(rows, null, 2) + "\n");
  }
}
