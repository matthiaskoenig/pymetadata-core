# pymetadata-core

A Rust core which reads and writes COMBINE archives (OMEX, a zip with a `manifest.xml`) lazily. It replaces the hot paths of the python library [pymetadata](https://github.com/matthiaskoenig/pymetadata) and ships as a python wheel and as a wasm module without imports. The design is in `docs/design/omex-core.md`.

## Crates

| crate | package | what it is |
| --- | --- | --- |
| `crates/core` | `pymetadata-core` | the archive, the manifest, the formats, the zip container |
| `crates/python` | `pymetadata-python` | the pyo3 binding, built into the python wheel `pymetadata-core` (module `pymetadata_core`), see `python/README.md` |
| `crates/wasm` | `pymetadata-wasm` | the raw C ABI host of the wasm module, which imports nothing |

## Checks

What continuous integration runs, so `--locked` here as well: the committed `Cargo.lock` is what is checked. `missing_docs` is a workspace lint, so every public item carries a doc comment and `clippy -D warnings` fails on an undocumented one.

The release build of the wasm module comes first, and not because `cargo test` needs it to compile: `crates/wasm/tests/imports.rs` reads the built module and asserts that it imports nothing, which is the property the hand-rolled zip container of this workspace exists for. The test fails when the module is missing, or older than a source, a manifest or the lock it was built from, rather than skipping, so a checkout which never built it has no passing `cargo test` to read as an answer. `cargo test` does not build another target by itself, which is why the line is here.

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build -p pymetadata-wasm --release --locked --target wasm32-unknown-unknown
cargo test --locked
cargo check --locked --workspace --exclude pymetadata-python --target wasm32-unknown-unknown
cargo check --locked --workspace --exclude pymetadata-python --target wasm32-wasip1
```

The python binding is not a default member of the workspace, so it has gates of its own. The wheel is built with maturin, installed into a uv environment next to `pytest` and `pymetadata` (the tests cross-check against it), and tested from there; `--no-sync` keeps `uv run` from rebuilding the package into the environment:

```bash
cargo clippy -p pymetadata-python --locked --all-targets -- -D warnings
uvx ruff@0.17.5 check
uvx ruff@0.17.5 format --check
uv venv
uv pip install pytest pymetadata
uv run --with 'maturin>=1.16,<2' maturin build --release --locked -o dist
uv pip install --force-reinstall dist/*.whl
uv run --no-sync pytest python/tests -q
```

`crates/core/src/formats.rs` and `crates/core/tests/data/formats.json` are generated from `pymetadata.omex` by `scripts/formats_table.py`; run it from a pymetadata checkout with `uv run python`.

## License

MIT
