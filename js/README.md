# @matthiaskoenig/pymetadata-core

Read and write COMBINE archives (OMEX) in the browser, with a WebAssembly module and no server. No dependency, and the module imports nothing, so there is no shim to set up. The archive is read lazily: opening one parses its directory, and a member is inflated when it is asked for.

```javascript
import { createRuntime } from "@matthiaskoenig/pymetadata-core";

// serve node_modules/@matthiaskoenig/pymetadata-core/runtime/pymetadata-core.wasm, the file the
// package exports as "./pymetadata-core.wasm", with the type application/wasm
const module = await WebAssembly.compileStreaming(fetch("/pymetadata-core.wasm"));
const omex = await createRuntime({ module });

omex.open(new Uint8Array(await file.arrayBuffer()));
const entries = omex.manifest(); // [{ location, format, master }, ...]
const model = omex.entry("./model.xml"); // Uint8Array
omex.setEntry("data.csv", null, false, new TextEncoder().encode("a,b\n1,2\n"));
omex.removeEntry("./README.md");
const bytes = omex.write(); // Uint8Array, members which did not change are copied as they are
```

`createRuntime({ module })` takes a compiled `WebAssembly.Module` or the bytes of the file and is the only asynchronous call. A runtime holds one archive; make one per Web Worker to work on several at once, and never call it from inside one of its own calls. Every array it returns is a copy which the page owns.

## Errors

A failed call throws an `OmexError` with a `kind` and a `message`:

| kind | meaning |
| --- | --- |
| `malformed` | the archive is not a readable archive |
| `refused` | a limit or a rule of the format, for instance a location outside of the archive |
| `unsupported` | the archive uses something 0.1.0 does not read, for instance a compression method |
| `not_found` | the location is not in the archive |
| `internal` | the request cannot be carried out, for instance a read before `open`; also any exception which is no trap of the module (a stack overflow, a memory which could not grow, a failure of the loader), and that one spends the instance |
| `panic` | a bug of the build, with the message of the panic |
| `trap` | the module trapped without a message, for instance an allocation it could not serve |
| `spent` | any call after a `panic` or a `trap` |

All but `panic`, `trap` and `spent` leave the runtime usable, apart from the exceptions named for `internal`. Any exception of a call spends the instance too, and says what it was: `trap` for a `WebAssembly.RuntimeError` without a message from the module, `internal` for any other exception, whether the module or the loader caused it. A `panic` or a `trap` leaves the instance in an unknown state: `runtime.spent` is then `true`, every later call throws `spent`, and the page creates a new runtime.

The module offers no way to clear its output buffer, so the loader replaces it with a harmless answer before every call and only reads a panic message from a trap of the call itself. A trap of that replacing call is reported as a plain `trap`, even when the module panicked, rather than risk reading what an earlier answer left, which may be bytes a member of the archive chose.

Two limitations of 0.1.0: `open` takes no password, so an encrypted archive cannot be opened, and a member which inflates to more than 1 GiB is `refused`.

## Developing

```bash
node build.mjs          # build the module into runtime/ and copy the test fixture
node build.mjs --check  # fail when the module is missing or older than anything under `crates/`, `Cargo.lock`, the root `Cargo.toml`, `rust-toolchain.toml` or `.cargo/config.toml`
npm test
npm pack --dry-run
```

`runtime/` and `test/fixtures/` are built and not committed: run `node build.mjs` before `npm test` on a fresh checkout, or the tests fail on the missing module and fixture. One test builds a second module with the `panic-probe` feature of `pymetadata-wasm` into `target/panic-probe`, which panics on purpose, to prove the panic path in the real engine; it needs `cargo` and the `wasm32-unknown-unknown` target (`rust-toolchain.toml` installs it).
