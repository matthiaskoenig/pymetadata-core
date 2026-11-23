//! The module a page loads imports nothing at all, and exports what a loader calls.
//!
//! The one reason this host targets `wasm32-unknown-unknown` with a raw C ABI rather than
//! `wasm32-wasip1` is that there is no C to link here and therefore nothing to import: a page
//! instantiates the module with an empty import object and needs neither a WASI shim nor a
//! memory of its own. An import which appears is that property lost, and a loader which passes
//! `{}` would fail with a `LinkError` instead. It is also what decided the dependencies of the
//! core: the `zip` crate pulls `getrandom/wasm_js` and with it `wasm-bindgen`, which the spike
//! measured at six imports, where `flate2` with `zlib-rs`, `roxmltree` and `crc32fast` have
//! none (`spike/RESULTS.md`).
//!
//! The model of this test is `the_module_imports_nothing` of `crates/wasm/tests/imports.rs` of
//! `pkpdutils-core`, with one difference: a module which is missing, or older than a file it was
//! built from, fails this test where the sibling skips it. A test which passes because nothing
//! was built would let an import creep in unnoticed, which is the one thing it is here to
//! prevent. This is why the release build of the module comes before `cargo test --locked` in
//! the `checks` block of the README and of the design document.
//!
//! Which files those are is half a list in this file and half cargo's: the sources come from the
//! dep-info file `pymetadata_wasm.d`, which cargo writes next to the module, so a module gains a
//! source here as soon as it gains one in the crate; the manifests and the lock are named below,
//! because a dependency bump which changes only `Cargo.lock`, which is how dependabot runs in
//! this project, changes what is linked into the module without touching a single source.
//! Comparing modification times is the rule cargo itself rebuilds by, so a module this test
//! calls stale after such a bump is one `cargo build` away from fresh.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How the module under test is built, named by every failure of this test.
///
/// `--locked` for the reason every gate of this workspace carries it: the remedy this test
/// advertises has to build against the committed lock and not against one cargo wrote for the
/// occasion, which would be a module of other dependencies than the ones under review.
const BUILD: &str =
    "cargo build -p pymetadata-wasm --release --locked --target wasm32-unknown-unknown";

/// The kind byte of an exported function, as the export section spells it.
const FUNCTION: u8 = 0;

/// The kind byte of an exported memory.
const MEMORY: u8 = 2;

/// The exports a loader calls, with their kind: the eleven functions of the ABI and the memory
/// it reads their output in.
///
/// A dropped `#[unsafe(no_mangle)]`, a renamed export or a linker flag which hides one would
/// otherwise pass every gate of this workspace and fail in the loader of the npm package. The
/// kind is asserted with the name, because a global or a table of the right name is no function
/// a page can call and would satisfy a check which only reads names.
const EXPORTS: [(&str, u8); 12] = [
    ("memory", MEMORY),
    ("omex_about", FUNCTION),
    ("omex_alloc", FUNCTION),
    ("omex_entry", FUNCTION),
    ("omex_manifest", FUNCTION),
    ("omex_new", FUNCTION),
    ("omex_open", FUNCTION),
    ("omex_output_len", FUNCTION),
    ("omex_output_ptr", FUNCTION),
    ("omex_remove_entry", FUNCTION),
    ("omex_set_entry", FUNCTION),
    ("omex_write", FUNCTION),
];

/// The root of the repository, which holds the lock and the crates.
///
/// Canonical where it can be, so that a failure names the files to look at.
fn repository() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::canonicalize(&root).unwrap_or(root)
}

/// Where `cargo build --target wasm32-unknown-unknown --release` puts the module.
///
/// `CARGO_TARGET_DIR` is honoured and a `build.target-dir` of `.cargo/config.toml` is not, which
/// the failure of a module which is not there says, so that nobody diagnoses a missing build
/// when the artifact is somewhere else.
fn module_path() -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR")
        .unwrap_or_else(|_| repository().join("target").to_string_lossy().into_owned());
    Path::new(&target).join("wasm32-unknown-unknown/release/pymetadata_wasm.wasm")
}

/// The id of the import section of a wasm module.
const IMPORT_SECTION: u8 = 2;

/// The id of the export section of a wasm module.
const EXPORT_SECTION: u8 = 7;

/// Read one LEB128 unsigned integer, and how many bytes it took.
fn leb128(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    let mut shift = 0;
    for (at, byte) in bytes.iter().enumerate() {
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at + 1));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

/// The payload of the section with this id, `None` when the module carries none.
///
/// A wasm module is the magic, the version and then a sequence of sections, each of them an id,
/// a LEB128 length and that many bytes.
fn section(module: &[u8], id: u8) -> Option<&[u8]> {
    assert_eq!(module.get(..4), Some(&b"\0asm"[..]), "no wasm magic");
    let mut at = 8;
    while at < module.len() {
        let kind = module[at];
        let (len, read) = leb128(&module[at + 1..]).expect("a section length");
        let payload = at + 1 + read;
        if kind == id {
            return module.get(payload..payload + len);
        }
        at = payload + len;
    }
    None
}

/// What the module exports, each as its name and its kind byte, in the order of the section.
///
/// An entry of that section is a name, a kind byte (a function, a table, a memory or a global)
/// and the LEB128 index of the thing itself, which is of no interest here.
fn exports(module: &[u8]) -> Vec<(String, u8)> {
    let Some(payload) = section(module, EXPORT_SECTION) else {
        return Vec::new();
    };
    let (count, read) = leb128(payload).expect("the number of exports");
    let mut at = read;
    let mut found = Vec::with_capacity(count);
    for _ in 0..count {
        let (len, read) = leb128(&payload[at..]).expect("the length of a name");
        at += read;
        let name = payload.get(at..at + len).expect("the name of an export");
        at += len;
        let kind = *payload.get(at).expect("the kind of an export");
        found.push((String::from_utf8_lossy(name).into_owned(), kind));
        at += 1 + leb128(&payload[at + 1..])
            .expect("the index of an export")
            .1;
    }
    found
}

/// The files the module is built from: cargo's sources, the manifests, the lock and the
/// toolchain.
///
/// `rust-toolchain.toml` pins the compiler, which decides what is in the module as much as a
/// dependency does; cargo rebuilds when it changes, because the version of rustc is part of
/// every fingerprint. `.cargo/config.toml` would carry the flags of a build and this repository
/// has none, so the path is listed for the day it gains one: a file which is not there is read
/// as a file which was not changed, see [`newer_than`].
fn inputs(module: &Path) -> Vec<PathBuf> {
    let root = repository();
    let mut files = vec![
        root.join("Cargo.lock"),
        root.join("Cargo.toml"),
        root.join("crates/wasm/Cargo.toml"),
        root.join("crates/core/Cargo.toml"),
        root.join("rust-toolchain.toml"),
        root.join(".cargo/config.toml"),
    ];
    files.extend(sources(module));
    files
}

/// The sources the module was built from, as cargo wrote them into its dep-info file.
///
/// The file is `make` syntax, one rule of the form `target: source source ...`, with a space as
/// the separator and a backslash in front of a space inside a path. A path of this repository
/// holds no space, so the escape is the one thing not handled here: a file whose name holds one
/// is read as two sources which do not exist, and a source which does not exist is not newer
/// than the module.
fn sources(module: &Path) -> Vec<PathBuf> {
    let path = module.with_extension("d");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} cannot be read ({error}), so it is not known what {} was built from; build it \
             with\n  {BUILD}",
            path.display(),
            module.display()
        )
    });
    let (_, listed) = text
        .split_once(": ")
        .unwrap_or_else(|| panic!("{} names no sources of the module: {text}", path.display()));
    listed.split_whitespace().map(PathBuf::from).collect()
}

/// The file which was changed after the module was built, if there is one.
fn newer_than(module: &Path, built: SystemTime) -> Option<PathBuf> {
    inputs(module).into_iter().find(|path| {
        std::fs::metadata(path)
            .and_then(|data| data.modified())
            .is_ok_and(|changed| changed > built)
    })
}

/// The module, which has to be there and has to be the one of the files it was built from.
fn module() -> (PathBuf, Vec<u8>) {
    let path = module_path();
    let module = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "{} cannot be read ({error}), so nothing of it was checked; build it with\n  \
             {BUILD}\nThat path is CARGO_TARGET_DIR or the target directory of the repository; a \
             build.target-dir of .cargo/config.toml is not read here.",
            path.display()
        )
    });
    let built = std::fs::metadata(&path)
        .and_then(|data| data.modified())
        .unwrap_or_else(|error| panic!("the time {} was built ({error})", path.display()));
    if let Some(changed) = newer_than(&path, built) {
        panic!(
            "{} is older than {}, so what it carries is another build's; build it again with\n  \
             {BUILD}\nA file whose content did not change does not make cargo relink, where one \
             which changed does; `touch crates/wasm/src/lib.rs` forces it.",
            path.display(),
            changed.display()
        );
    }
    (path, module)
}

/// A module of two sections, the second of which imports one function.
///
/// Hand built, because the one way the test below can be wrong is a section walker which finds
/// no import section in a module which has one, and no module of this workspace has one to try
/// it against. The bytes are the magic, the version, a type section of `() -> ()` and an import
/// section of one entry, `e.f` of that type, which node reads as one function import.
const WITH_AN_IMPORT: [u8; 23] = [
    0x00, 0x61, 0x73, 0x6d, // "\0asm"
    0x01, 0x00, 0x00, 0x00, // version 1
    0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type section: one type, () -> ()
    0x02, 0x07, 0x01, 0x01, 0x65, 0x01, 0x66, 0x00, 0x00, // import section: "e"."f", func 0
];

/// A module of a memory section and an export section which exports it as `mem`.
///
/// Hand built for the same reason, and valid: node reads it as one memory export. It is what
/// shows that [`exports`] reads the name of an export and not the bytes around it.
const WITH_AN_EXPORT: [u8; 22] = [
    0x00, 0x61, 0x73, 0x6d, // "\0asm"
    0x01, 0x00, 0x00, 0x00, // version 1
    0x05, 0x03, 0x01, 0x00, 0x01, // memory section: one memory, one page at least
    0x07, 0x07, 0x01, 0x03, 0x6d, 0x65, 0x6d, 0x02, 0x00, // export section: "mem", memory 0
];

#[test]
fn an_import_section_is_found_where_there_is_one() {
    let found = section(&WITH_AN_IMPORT, IMPORT_SECTION).map(<[u8]>::len);
    assert_eq!(found, Some(7));
    // and the walker reads the sections in front of it rather than the first byte it sees
    assert_eq!(section(&WITH_AN_IMPORT[..14], IMPORT_SECTION), None);
    assert_eq!(section(&WITH_AN_EXPORT, IMPORT_SECTION), None);
}

#[test]
fn an_export_is_read_by_its_name_and_its_kind() {
    assert_eq!(exports(&WITH_AN_EXPORT), vec![("mem".to_string(), MEMORY)]);
    assert!(exports(&WITH_AN_IMPORT).is_empty());
}

#[test]
fn the_module_imports_nothing() {
    let (path, module) = module();
    match section(&module, IMPORT_SECTION).map(<[u8]>::len) {
        None => (),
        // an empty section is a section of one zero byte, the count of its entries
        Some(1) => (),
        Some(len) => panic!(
            "{} carries an import section of {len} bytes; a page instantiates this module with \
             an empty import object",
            path.display()
        ),
    }
}

#[test]
fn the_module_exports_the_abi_and_its_memory() {
    let (path, module) = module();
    let found = exports(&module);
    for (name, kind) in EXPORTS {
        assert!(
            found
                .iter()
                .any(|export| export == &(name.to_string(), kind)),
            "{} does not export {name} as kind {kind}, it exports {found:?}",
            path.display()
        );
    }
    // and nothing else: the `panic-probe` export panics on demand and ships in no module of a
    // release, so a default build which carries it has enabled the feature by accident
    assert_eq!(
        found.len(),
        EXPORTS.len(),
        "{} exports more than the ABI: {found:?}",
        path.display()
    );
    assert!(
        !found.iter().any(|(name, _)| name == "omex_panic_probe"),
        "{} carries the panic probe",
        path.display()
    );
}
