// Build the wasm module into js/runtime/, copy the test fixture, and print the size.
//
//   node js/build.mjs           build and copy
//   node js/build.mjs --check   fail when the module is missing or older than its sources
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

import { root } from "./version.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const built = join(root, "target/wasm32-unknown-unknown/release/pymetadata_wasm.wasm");
const runtime = join(here, "runtime/pymetadata-core.wasm");
const corpus = join(root, "crates/core/tests/data/omex/CompModels.omex");
const fixture = join(here, "test/fixtures/CompModels.omex");

/** The newest modification time under `directory`, in milliseconds. */
function newest(directory) {
  let latest = 0;
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    latest = Math.max(latest, entry.isDirectory() ? newest(path) : statSync(path).mtimeMs);
  }
  return latest;
}

if (process.argv.includes("--check")) {
  if (!existsSync(runtime)) {
    console.error(`${runtime} is missing; run: node js/build.mjs`);
    process.exit(1);
  }
  // The inputs of the module. Deliberately the same set as `inputs()` of
  // crates/wasm/tests/imports.rs, which the rust gate checks the module against: the manifests,
  // the lock and the toolchain decide what is linked as much as a source does, and the profile
  // of the root Cargo.toml moved the size more than any source. Keep the two in step.
  const files = ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"];
  const sources = Math.max(
    newest(join(root, "crates")),
    ...files.map((file) => (existsSync(join(root, file)) ? statSync(join(root, file)).mtimeMs : 0)),
  );
  if (statSync(runtime).mtimeMs < sources) {
    console.error(`${runtime} is older than crates/, the lock, a root manifest or the toolchain file; run: node js/build.mjs`);
    process.exit(1);
  }
  console.log(`${runtime} is current`);
} else {
  execFileSync(
    "cargo",
    ["build", "-p", "pymetadata-wasm", "--release", "--locked", "--target", "wasm32-unknown-unknown"],
    { cwd: root, stdio: "inherit" },
  );
  mkdirSync(dirname(runtime), { recursive: true });
  mkdirSync(dirname(fixture), { recursive: true });
  copyFileSync(built, runtime);
  copyFileSync(corpus, fixture);
  const bytes = readFileSync(runtime);
  console.log(`${runtime}: ${bytes.length} bytes, ${gzipSync(bytes, { level: 9 }).length} gzipped`);
}
