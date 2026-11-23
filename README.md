# pymetadata-core

A Rust core which reads and writes COMBINE archives (OMEX, a zip with a `manifest.xml`) lazily. It replaces the hot paths of the python library [pymetadata](https://github.com/matthiaskoenig/pymetadata) and ships as a python wheel and as a wasm module without imports. The design is in `docs/design/omex-core.md`.

## Crates

| crate | package | what it is |
| --- | --- | --- |
| `crates/core` | `pymetadata-core` | the archive, the manifest, the formats, the zip container |

## Checks

What continuous integration runs, so `--locked` here as well: the committed `Cargo.lock` is what is checked. `missing_docs` is a workspace lint, so every public item carries a doc comment and `clippy -D warnings` fails on an undocumented one.

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

`crates/core/src/formats.rs` and `crates/core/tests/data/formats.json` are generated from `pymetadata.omex` by `scripts/formats_table.py`; run it from a pymetadata checkout with `uv run python`.

## License

MIT
