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

The JavaScript package under `js/` has its own gates. `node js/build.mjs` comes before the tests, because the module and the fixture archive are built, not committed, and a fresh checkout fails the tests without them. One test builds a second module with the `panic-probe` feature, so it needs `cargo` and the `wasm32-unknown-unknown` target:

```bash
node js/build.mjs
(cd js && npm test && npm pack --dry-run)
```

The python binding is not a default member of the workspace, so it has gates of its own. `uv.lock` pins the development environment, as it does in `pymetadata`: `uv sync --locked` creates it from the lock (and fails instead of re-resolving when the lock no longer matches `pyproject.toml`), builds the extension into it and installs `pytest`, `pymetadata` (the tests cross-check against it) and `ruff`. `uv lock --check` verifies the lock alone. The compiled extension lands in `python/pymetadata_core/` and is gitignored.

```bash
cargo clippy -p pymetadata-python --locked --all-targets -- -D warnings
cargo test -p pymetadata-python --locked
uv sync --locked
uv run --no-sync ruff check
uv run --no-sync ruff format --check
uvx ty@0.0.108 check
uv run --no-sync pytest python/tests -q
```

The wheel which ships is a different job: it is built with maturin from the locked sources, installed over the development build and tested as installed; `--no-sync` keeps `uv run` from rebuilding the package into the environment:

```bash
uv run --with 'maturin>=1.16,<2' maturin build --release --locked -o dist
uv pip install --force-reinstall dist/*.whl
uv run --no-sync pytest python/tests -q
uv run --no-sync --with 'maturin>=1.16,<2' maturin build --release --sdist -o target/sdist
```

The last line builds the source distribution of the release; it takes no `--locked`, since the workspace of an sdist is not the one `Cargo.lock` was resolved for.

Continuous integration (`.github/workflows/ci.yml`) runs exactly these lists as the jobs `checks`, `wasm`, `js`, `python`, `ruff` and `ty`, which are the required checks of `.github/rulesets/develop.json`. How a release is made is in `docs/release.md`.

`crates/core/src/formats.rs` and `crates/core/tests/data/formats.json` are generated from `pymetadata.omex` by `scripts/formats_table.py`; run it from a pymetadata checkout with `uv run python`.

## License

MIT
