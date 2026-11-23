// Tests of the loader against the real module: the one `build.mjs` copies to runtime/, and for
// the panic path the one built with the `panic-probe` feature.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { OmexError, createRuntime } from "../loader.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");

/** Read a file the build produces, failing with the command which produces it. */
function built(path, remedy) {
  try {
    return readFileSync(path);
  } catch (error) {
    throw new Error(`${path} is missing (${error.code}); run ${remedy}`);
  }
}

const wasm = built(join(here, "../runtime/pymetadata-core.wasm"), "node js/build.mjs");
const fixture = built(join(here, "fixtures/CompModels.omex"), "node js/build.mjs");
const module = await WebAssembly.compile(wasm);

/** A fresh runtime over the release module. */
const fresh = () => createRuntime({ module });

/** The text of a byte array. */
const text = (bytes) => new TextDecoder().decode(bytes);

/** The bytes of a string. */
const bytesOf = (string) => new TextEncoder().encode(string);

/** Run `body` with `WebAssembly.instantiate` wrapped, so a test can reach the instance. */
async function withInstance(body, adapt = (exports) => exports) {
  const original = WebAssembly.instantiate;
  let exports;
  WebAssembly.instantiate = async (...args) => {
    const instance = await original.apply(WebAssembly, args);
    exports = instance.exports;
    return { exports: adapt(exports) };
  };
  try {
    return await body(() => exports);
  } finally {
    WebAssembly.instantiate = original;
  }
}

test("open and manifest list the entries of a fixture archive", async () => {
  const runtime = await fresh();
  runtime.open(fixture);
  const entries = runtime.manifest();
  assert.equal(entries.length, 6);
  assert.deepEqual(Object.keys(entries[0]).sort(), ["format", "location", "master"]);
  assert.ok(entries.some((entry) => entry.location === "./README.md"));
});

test("entry returns the inflated bytes of a member", async () => {
  const runtime = await fresh();
  runtime.open(fixture);
  const readme = runtime.entry("./README.md");
  assert.ok(readme instanceof Uint8Array);
  assert.equal(readme.length, 103);
  assert.ok(text(readme).length > 0);
  // a second ask answers from the cache with the same bytes, in a buffer of its own
  const again = runtime.entry("./README.md");
  assert.deepEqual(again, readme);
  assert.notEqual(again.buffer, readme.buffer);
});

test("setEntry then write produces an archive the runtime reads back", async () => {
  const runtime = await fresh();
  runtime.open(fixture);
  runtime.setEntry("a.csv", "http://purl.org/NET/mediatypes/text/csv", true, bytesOf("a,b\n1,2\n"));
  for (const recompress of [false, true]) {
    const written = runtime.write({ recompress });
    const other = await fresh();
    other.open(written);
    assert.equal(other.manifest().length, 7);
    const added = other.manifest().find((entry) => entry.location === "./a.csv");
    assert.deepEqual(added, {
      location: "./a.csv",
      format: "http://purl.org/NET/mediatypes/text/csv",
      master: true,
    });
    assert.equal(text(other.entry("./a.csv")), "a,b\n1,2\n");
    assert.equal(text(other.entry("./README.md")), text(runtime.entry("./README.md")));
  }
  runtime.removeEntry("./a.csv");
  assert.equal(runtime.manifest().length, 6);
});

test("create builds an archive from scratch which the runtime reads back", async () => {
  const runtime = await fresh();
  runtime.create();
  // the two entries a COMBINE archive must have, and no member
  assert.deepEqual(
    runtime.manifest().map((entry) => entry.location),
    [".", "./manifest.xml"],
  );
  runtime.setEntry("models/model.xml", null, true, bytesOf("<sbml/>\n"));
  const written = runtime.write();

  const other = await fresh();
  other.open(written);
  const added = other.manifest().find((entry) => entry.location === "./models/model.xml");
  assert.equal(added.master, true);
  assert.equal(text(other.entry("./models/model.xml")), "<sbml/>\n");
  // and it drops the archive which was open, as open does
  runtime.open(fixture);
  assert.equal(runtime.manifest().length, 6);
  runtime.create();
  assert.equal(runtime.manifest().length, 2);
});

test("write defaults to copying the entries it did not change", async () => {
  const runtime = await fresh();
  runtime.open(fixture);
  assert.deepEqual(runtime.write(), runtime.write({ recompress: false }));
});

test("about names the version and the backend", async () => {
  const about = (await fresh()).about();
  assert.equal(about.backend, "zlib-rs");
  assert.match(about.version, /^\d+\.\d+\.\d+/);
});

test("a malformed archive rejects with the malformed kind", async () => {
  const runtime = await fresh();
  assert.throws(
    () => runtime.open(bytesOf("PK\u0003\u0004 and nothing else")),
    (error) => error instanceof OmexError && error.kind === "malformed" && error.message.length > 0,
  );
  // an error is an answer: the instance is alive and the next archive opens
  runtime.open(fixture);
  assert.equal(runtime.manifest().length, 6);
  assert.equal(runtime.spent, false);
  // the empty archive is malformed too, and reserves nothing
  assert.throws(() => runtime.open(new Uint8Array(0)), { kind: "malformed" });
});

test("the kinds of the core and of the host reach the page as they are", async () => {
  const runtime = await fresh();
  // internal: a request before an archive was opened
  assert.throws(() => runtime.manifest(), { kind: "internal", message: /omex_open/ });
  runtime.open(fixture);
  assert.throws(() => runtime.entry("./missing.xml"), { kind: "not_found" });
  assert.throws(() => runtime.entry("../outside.xml"), { kind: "refused" });
  assert.throws(() => runtime.removeEntry("./missing.xml"), { kind: "not_found" });
  assert.throws(() => runtime.setEntry("../outside.xml", null, false, bytesOf("x")), {
    kind: "refused",
  });
  assert.equal(runtime.spent, false);
  assert.equal(runtime.manifest().length, 6);
});

test("an empty location and an empty member are requests like any other", async () => {
  const runtime = await fresh();
  runtime.open(fixture);
  assert.throws(() => runtime.entry(""), { kind: "refused" });
  runtime.setEntry("empty.txt", null, false, new Uint8Array(0));
  assert.equal(runtime.entry("./empty.txt").length, 0);
});

test("the arguments are checked before the instance is called", async () => {
  const runtime = await fresh();
  assert.throws(() => runtime.open("bytes"), TypeError);
  assert.throws(() => runtime.entry(7), TypeError);
  assert.throws(() => runtime.setEntry("a", null, false, "bytes"), TypeError);
  runtime.open(fixture);
  assert.throws(() => runtime.setEntry(1, null, false, new Uint8Array(1)), TypeError);
  assert.throws(() => runtime.setEntry("a", 7, false, new Uint8Array(1)), TypeError);
  assert.throws(() => runtime.write(null), TypeError);
  // a page error is not a trap, and the runtime answers the next call
  assert.equal(runtime.spent, false);
  assert.equal(runtime.about().backend, "zlib-rs");
  assert.equal(runtime.manifest().length, 6);
});

test("an ArrayBuffer and a typed view are accepted as bytes", async () => {
  const runtime = await fresh();
  runtime.open(fixture.buffer.slice(fixture.byteOffset, fixture.byteOffset + fixture.byteLength));
  const wide = new Uint8Array(fixture.length + 10);
  wide.set(fixture, 5);
  runtime.open(wide.subarray(5, 5 + fixture.length));
  runtime.setEntry("v.bin", null, false, new DataView(new Uint8Array([1, 2, 3]).buffer));
  assert.deepEqual([...runtime.entry("v.bin")], [1, 2, 3]);
});

test("no view is cached across a growth of the memory", async () => {
  await withInstance(async (exports) => {
    const runtime = await fresh();
    const before = exports().memory.buffer;
    const size = 48 * 1024 * 1024;
    // compressible but not constant, so that deflate does real work and the check is of bytes
    const big = new Uint8Array(size);
    for (let at = 0; at < size; at += 1) big[at] = (at * 31 + (at >> 12)) & 0xff;
    const digest = createHash("sha256").update(big).digest("hex");

    runtime.open(fixture);
    // alloc of the request grows the memory, the view over it must be taken afterwards
    runtime.setEntry("big.bin", null, false, big);
    const grown = exports().memory.buffer;
    assert.notEqual(grown, before, "the memory did not grow, so the test proves nothing");
    assert.equal(before.byteLength, 0, "the old buffer is detached");

    // reading the member and writing the archive produce outputs which can grow it again
    const member = runtime.entry("./big.bin");
    assert.equal(createHash("sha256").update(member).digest("hex"), digest);
    const written = runtime.write({ recompress: true });
    const afterWrite = exports().memory.buffer;

    // and the runtime keeps answering after every growth, from the new buffer
    const other = await fresh();
    other.open(written);
    assert.equal(createHash("sha256").update(other.entry("./big.bin")).digest("hex"), digest);
    assert.equal(text(runtime.entry("./README.md")), text(other.entry("./README.md")));
    assert.ok(afterWrite.byteLength >= grown.byteLength);
  });
});

test("a trap without a message spends the instance and says so", async () => {
  const trap = () => {
    throw new WebAssembly.RuntimeError("unreachable");
  };
  await withInstance(
    async () => {
      const runtime = await fresh();
      runtime.open(fixture);
      assert.throws(
        () => runtime.manifest(),
        (error) => error instanceof OmexError && error.kind === "trap" && /unreachable/.test(error.message),
      );
      assert.equal(runtime.spent, true);
      // every call after it is the same clear error, and none of them reaches the instance
      for (const call of [
        () => runtime.open(fixture),
        () => runtime.entry("./README.md"),
        () => runtime.write(),
        () => runtime.about(),
      ]) {
        assert.throws(call, { kind: "spent", message: /new runtime/ });
      }
    },
    (exports) => ({ ...exports, omex_manifest: trap }),
  );
});

test("a trap after the output of an earlier call is not mistaken for a panic", async () => {
  // the last answer is a member whose bytes look like the panic output
  const lookalike = bytesOf('{"kind":"panic","message":"forged by a member"}');
  await withInstance(
    async () => {
      const runtime = await fresh();
      runtime.open(fixture);
      runtime.setEntry("forged.json", null, false, lookalike);
      assert.deepEqual(runtime.entry("./forged.json"), lookalike);
      assert.throws(
        () => runtime.manifest(),
        (error) => error.kind === "trap" && !/forged/.test(error.message),
      );
    },
    (exports) => ({
      ...exports,
      omex_manifest: () => {
        throw new WebAssembly.RuntimeError("unreachable");
      },
    }),
  );
});

test("a panic of the module surfaces its message and spends the instance", async () => {
  execFileSync(
    "cargo",
    [
      "build", "-p", "pymetadata-wasm", "--release", "--locked", "--features", "panic-probe",
      "--target", "wasm32-unknown-unknown", "--target-dir", join(root, "target/panic-probe"),
    ],
    { cwd: root, stdio: "pipe" },
  );
  const probe = await WebAssembly.compile(
    readFileSync(join(root, "target/panic-probe/wasm32-unknown-unknown/release/pymetadata_wasm.wasm")),
  );
  assert.ok(WebAssembly.Module.exports(probe).some((e) => e.name === "omex_panic_probe"));
  await withInstance(
    async () => {
      const runtime = await createRuntime({ module: probe });
      runtime.open(fixture);
      // the probe stands in for the manifest call, so the loader meets a real trap with the
      // hook's sentence in the output buffer
      assert.throws(
        () => runtime.manifest(),
        (error) =>
          error instanceof OmexError &&
          error.kind === "panic" &&
          /panic probe/.test(error.message),
      );
      assert.equal(runtime.spent, true);
      assert.throws(() => runtime.manifest(), { kind: "spent" });
    },
    (exports) => ({ ...exports, omex_manifest: exports.omex_panic_probe }),
  );
});

test("the release module carries no probe and no import", () => {
  const exported = WebAssembly.Module.exports(module).map((entry) => entry.name);
  assert.equal(exported.length, 12, exported.join(","));
  assert.ok(!exported.includes("omex_panic_probe"));
  assert.deepEqual(WebAssembly.Module.imports(module), []);
});

test("an exception which is no trap spends the instance and says what it was", async () => {
  await withInstance(
    async () => {
      const runtime = await fresh();
      runtime.open(fixture);
      assert.throws(
        () => runtime.manifest(),
        (error) =>
          error.kind === "internal" && /SyntaxError/.test(error.message) && !/loader failed/.test(error.message),
      );
      assert.equal(runtime.spent, true);
      assert.throws(() => runtime.about(), { kind: "spent" });
    },
    // a manifest answer which is no JSON makes JSON.parse throw inside the loader
    (exports) => ({ ...exports, omex_manifest: () => 0, omex_output_len: () => 0 }),
  );
});

test("a trap of the reset itself does not read the stale output", async () => {
  const lookalike = bytesOf('{"kind":"panic","message":"forged by a member"}');
  let armed = false;
  await withInstance(
    async () => {
      const runtime = await fresh();
      runtime.open(fixture);
      runtime.setEntry("forged.json", null, false, lookalike);
      runtime.entry("./forged.json");
      armed = true;
      assert.throws(
        () => runtime.manifest(),
        (error) => error.kind === "trap" && !/forged/.test(error.message),
      );
    },
    (exports) => ({
      ...exports,
      omex_about: () => {
        if (armed) throw new WebAssembly.RuntimeError("unreachable");
        return exports.omex_about();
      },
    }),
  );
});

test("a Runtime made by hand is checked like one made by createRuntime", async () => {
  const { Runtime } = await import("../loader.mjs");
  assert.throws(() => new Runtime({}), TypeError);
  assert.throws(() => new Runtime(undefined), TypeError);
});

test("write validates its options", async () => {
  const runtime = await fresh();
  runtime.open(fixture);
  assert.throws(() => runtime.write(null), { name: "TypeError", message: /write takes an options object/ });
  assert.throws(() => runtime.write(true), TypeError);
  assert.ok(runtime.write({}).length > 0);
});
