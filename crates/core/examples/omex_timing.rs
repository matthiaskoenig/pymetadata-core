//! Timings of the archive: the Rust side of `BENCHMARKS.md`.
//!
//! A benchmark is not a gate: nothing here is asserted and nothing runs in `cargo test`.
//!
//! ```text
//! cargo run --release --example omex_timing                    # markdown table
//! cargo run --release --example omex_timing -- --csv out.csv   # also the raw numbers
//! cargo run --release --example omex_timing -- [--csv out.csv] [directory of .omex files]
//! ```
//!
//! The protocol is the one of `spike/python_timing.py` and of `scripts/python_timing.py`: 3
//! warm-up calls, then 3 rounds of 20 timed calls, and the number is the median of the three
//! round medians. Every cell returns a checksum which is printed next to the number and passed
//! through [`black_box`], so the work cannot be optimized away and a reader can compare the
//! checksums of two runs.
//!
//! Every cell starts from the bytes of the archive in memory and includes `Archive::from_bytes`,
//! because the archive is lazy and a cell which skipped it would time nothing. No cell touches
//! the file system.

use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pymetadata_core::zip::Reader;
use pymetadata_core::{Archive, Entry, Manifest, OmexError};

const WARMUP: usize = 3;
const ROUNDS: usize = 3;
const CALLS: usize = 20;
/// The level `pymetadata.Omex.to_omex` deflates at, so that the recompressing write is like for
/// like with it.
const LEVEL: u32 = 9;
/// The default level of zlib and of python's `zipfile`, which is what the spike measured.
const LEVEL_DEFAULT: u32 = 6;
/// How many bytes of a member `classify_heads` reads, which is what the format of an XML
/// member is told from.
const HEAD: usize = 256;

/// The size of the content `modify_write` puts in place of one entry.
const PAYLOAD: usize = 64 * 1024;
const MANIFEST: &str = "./manifest.xml";

/// The median of a slice of timings.
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        f64::midpoint(values[n / 2 - 1], values[n / 2])
    }
}

/// One measured cell: the median of the round medians, the medians and the checksum.
struct Cell {
    ms: f64,
    rounds: [f64; ROUNDS],
    checksum: u64,
}

/// Time `f` by the protocol.
fn time(mut f: impl FnMut() -> Result<u64, OmexError>) -> Result<Cell, OmexError> {
    time_with(|| (), |()| f())
}

/// Time `f` by the protocol, with the input of every call made by `setup` before the clock
/// starts. The call owns its input, and what it drops is timed.
fn time_with<T>(
    mut setup: impl FnMut() -> T,
    mut f: impl FnMut(T) -> Result<u64, OmexError>,
) -> Result<Cell, OmexError> {
    let mut checksum = 0;
    for _ in 0..WARMUP {
        checksum = black_box(f(setup())?);
    }
    let mut rounds = [0.0; ROUNDS];
    for round in &mut rounds {
        let mut calls = Vec::with_capacity(CALLS);
        for _ in 0..CALLS {
            let input = setup();
            let start = Instant::now();
            checksum = black_box(f(input)?);
            calls.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        *round = median(&mut calls);
    }
    let ms = median(&mut rounds.to_vec());
    Ok(Cell {
        ms,
        rounds,
        checksum,
    })
}

/// The locations which have bytes of their own: not the archive (`.`) and not the manifest.
fn content_locations(archive: &Archive) -> Vec<String> {
    archive
        .locations()
        .filter(|location| *location != "." && *location != MANIFEST)
        .map(str::to_string)
        .collect()
}

/// `manifest_only`: open the archive and read its manifest. Nothing else is inflated.
fn manifest_only(bytes: &[u8]) -> Result<u64, OmexError> {
    let archive = Archive::from_bytes(bytes)?;
    Ok(archive.manifest().entries().len() as u64)
}

/// `manifest_owned`: the manifest of an archive whose bytes the caller already owns, so there is
/// no copy of the input. The zip reader takes the `Vec`, the manifest member is inflated and
/// parsed. It is not `Archive::from_bytes` minus the copy: it also skips the pass which adds the
/// members the manifest does not list, so it counts only what the manifest lists.
fn manifest_owned(bytes: Vec<u8>) -> Result<u64, OmexError> {
    let reader = Reader::new(bytes)?;
    let member = reader
        .members()
        .iter()
        .find(|member| member.name == "manifest.xml")
        .ok_or_else(|| OmexError::Malformed("archive has no manifest.xml".to_string()))?;
    let xml = reader.inflate(member)?;
    let xml = std::str::from_utf8(&xml)
        .map_err(|error| OmexError::Malformed(format!("manifest.xml is not UTF-8: {error}")))?;
    Ok(Manifest::parse(xml)?.entries().len() as u64)
}

/// `classify_heads`: open the archive and read the first [`HEAD`] bytes of every entry which
/// has bytes, which is what a host does to tell the format of a member from its content. It is
/// `read_all` with [`Archive::head`] in place of `Archive::entry_bytes`, so the two cells are
/// the cost of the same walk with and without inflating every member whole.
fn classify_heads(bytes: &[u8]) -> Result<u64, OmexError> {
    let mut archive = Archive::from_bytes(bytes)?;
    let mut total = 0;
    for location in content_locations(&archive) {
        match archive.head(&location, HEAD) {
            Ok(data) => total += data.len() as u64,
            // the manifest lists it and the zip does not hold it, as in iCGB21FR.omex
            Err(OmexError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(total)
}

/// `probe`: answer whether the bytes are a COMBINE archive, from the central directory alone.
///
/// This is the question a host asks in front of opening an archive. It parses the records at
/// the end and walks the central directory for the name of the manifest, and inflates nothing,
/// so the comparison is against `manifest_only`, which is the cheapest cell that opens one.
fn probe_container(bytes: &[u8]) -> Result<u64, OmexError> {
    Ok(u64::from(
        pymetadata_core::zip::probe::declares_manifest_in(bytes)?,
    ))
}

/// `read_all`: open the archive and inflate every entry which has bytes. An entry which the
/// manifest lists and the zip does not hold has none and is skipped.
fn read_all(bytes: &[u8]) -> Result<u64, OmexError> {
    let mut archive = Archive::from_bytes(bytes)?;
    let mut total = 0;
    for location in content_locations(&archive) {
        match archive.entry_bytes(&location) {
            Ok(data) => total += data.len() as u64,
            // the manifest lists it and the zip does not hold it, as in iCGB21FR.omex
            Err(OmexError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(total)
}

/// `modify_write`: open the archive, replace one entry with `payload`, and write it with the
/// copy of every entry which did not change.
fn modify_write(bytes: &[u8], target: &Entry, payload: &[u8]) -> Result<u64, OmexError> {
    let mut archive = Archive::from_bytes(bytes)?;
    archive.set_entry(target.clone(), payload.to_vec())?;
    Ok(archive.to_bytes()?.len() as u64)
}

/// `write_copy`: open the archive and write it again, nothing changed.
fn write_copy(bytes: &[u8]) -> Result<u64, OmexError> {
    let mut archive = Archive::from_bytes(bytes)?;
    Ok(archive.to_bytes()?.len() as u64)
}

/// `write_recompress`: open the archive and write it with every member inflated and deflated at
/// `level`.
fn write_recompress(bytes: &[u8], level: u32) -> Result<u64, OmexError> {
    let mut archive = Archive::from_bytes(bytes)?;
    Ok(archive.to_bytes_recompressed(level)?.len() as u64)
}

/// `input_copy`: the copy of the input which `Archive::from_bytes` makes, and nothing else. It is
/// part of every other cell, and is here so a reader can see how much.
fn input_copy(bytes: &[u8]) -> u64 {
    black_box(bytes.to_vec()).len() as u64
}

/// The archives of `directory`, sorted by name.
fn archives(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "omex")
        })
        .collect();
    paths.sort();
    paths
}

/// One row of the output.
struct Row {
    name: String,
    size: usize,
    entries: u64,
    op: &'static str,
    cell: Cell,
}

/// Time every cell of one archive.
fn measure(path: &Path) -> Result<Vec<Row>, OmexError> {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut probe = Archive::from_bytes(&bytes)?;
    let entries = probe.manifest().entries().len() as u64;
    // replace the first entry which has bytes, with the format and the master flag it has
    let target = content_locations(&probe)
        .iter()
        .find(|location| probe.entry_bytes(location).is_ok())
        .and_then(|location| probe.entry(location).cloned())
        .ok_or_else(|| {
            OmexError::NotFound(format!("{name} holds no entry besides the manifest"))
        })?;
    let payload = vec![b'x'; PAYLOAD];

    // not timed: the written archive is one a reader opens again, with the entries it should have
    let mut check = Archive::from_bytes(&bytes)?;
    check.set_entry(target.clone(), payload.clone())?;
    let written = Archive::from_bytes(&check.to_bytes()?)?;
    if written.manifest().entries().len() as u64 != entries {
        return Err(OmexError::Malformed(format!(
            "{name}: modify_write lost an entry"
        )));
    }

    let mut rows = Vec::new();
    let mut push = |op: &'static str, cell: Cell| {
        rows.push(Row {
            name: name.clone(),
            size: bytes.len(),
            entries,
            op,
            cell,
        });
    };
    push("manifest_only", time(|| manifest_only(&bytes))?);
    push(
        "manifest_owned",
        time_with(|| bytes.clone(), manifest_owned)?,
    );
    push("read_all", time(|| read_all(&bytes))?);
    push("classify_heads", time(|| classify_heads(&bytes))?);
    push("probe", time(|| probe_container(&bytes))?);
    push(
        "modify_write",
        time(|| modify_write(&bytes, &target, &payload))?,
    );
    push("write_copy", time(|| write_copy(&bytes))?);
    push(
        "write_recompress",
        time(|| write_recompress(&bytes, LEVEL))?,
    );
    push(
        "write_recompress_l6",
        time(|| write_recompress(&bytes, LEVEL_DEFAULT))?,
    );
    push("input_copy", time(|| Ok(input_copy(&bytes)))?);
    Ok(rows)
}

/// The size of an archive, for a person.
fn size(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{} kB", bytes.div_ceil(1000))
    }
}

fn main() {
    let mut csv = None;
    let mut directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/omex");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--csv" {
            csv = Some(args.next().expect("--csv takes a path"));
        } else {
            directory = PathBuf::from(arg);
        }
    }

    let mut rows = Vec::new();
    for path in archives(&directory) {
        match measure(&path) {
            Ok(measured) => rows.extend(measured),
            Err(error) => panic!("{}: {error}", path.display()),
        }
    }

    println!("| archive | size | entries | op | rust ms | checksum |");
    println!("|---|---|---|---|---|---|");
    for row in &rows {
        println!(
            "| {} | {} | {} | {} | {:.4} | {} |",
            row.name,
            size(row.size),
            row.entries,
            row.op,
            row.cell.ms,
            row.cell.checksum
        );
    }

    if let Some(path) = csv {
        let mut out = String::from(
            "archive,size_bytes,entries,op,ms,round1_ms,round2_ms,round3_ms,checksum\n",
        );
        for row in &rows {
            let _ = writeln!(
                out,
                "{},{},{},{},{:.4},{:.4},{:.4},{:.4},{}",
                row.name,
                row.size,
                row.entries,
                row.op,
                row.cell.ms,
                row.cell.rounds[0],
                row.cell.rounds[1],
                row.cell.rounds[2],
                row.cell.checksum
            );
        }
        fs::write(&path, out).unwrap_or_else(|error| panic!("cannot write {path}: {error}"));
    }
}
