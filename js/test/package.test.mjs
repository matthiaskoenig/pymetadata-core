// Tests of the package around the loader: its version and what it may touch.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { crateVersion } from "../version.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const read = (path) => readFileSync(join(here, "..", path), "utf8");

test("package.json version equals the crate version", () => {
  const pkg = JSON.parse(read("package.json"));
  assert.equal(pkg.version, crateVersion());
  assert.equal(pkg.name, "@matthiaskoenig/pymetadata-core");
  assert.equal(pkg.dependencies, undefined);
});

test("the loader touches no Node API", () => {
  // comments are prose and may name what the code does not use
  const source = read("loader.mjs")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "");
  for (const forbidden of [/\bnode:/, /\brequire\s*\(/, /\bimport\b/, /\bprocess\b/, /\bBuffer\b/, /__dirname/]) {
    assert.doesNotMatch(source, forbidden);
  }
});
