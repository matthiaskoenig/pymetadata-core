//! Recognizing a COMBINE archive by its structure, without reading the whole file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use pymetadata_core::zip::probe::{TAIL_SIZE, central_directory, declares_manifest};
use pyo3::prelude::*;

/// Whether the file at `path` is a COMBINE archive, i.e. a zip container with a manifest.
///
/// This reads the end of the file, which holds the record saying where the central directory
/// is, and then that directory, which lists the names. Nothing else is read: no member is
/// inflated and none is decrypted, so the cost is the size of the central directory and not
/// the size of the archive, where `Archive.from_bytes` needs every byte of it in memory.
///
/// A file which is no zip container, and a container which declares no manifest, answer
/// `False` rather than raising: this is the question "is this an archive at all", and `False`
/// is the answer for everything which is not one. An encrypted archive answers `True`, since
/// the manifest is declared whether or not a caller may read it, and so does a container with
/// a member name `Archive.from_bytes` would refuse: recognizing an archive is not opening one.
///
/// # Errors
///
/// `OSError` if the file cannot be opened or read, as `zipfile.is_zipfile` raises for a path
/// which is not there. Being unable to read a file is not an answer about its content.
#[pyfunction]
#[pyo3(text_signature = "(path)")]
pub fn has_manifest(py: Python<'_>, path: PathBuf) -> PyResult<bool> {
    // the file is read without the GIL, as every call of the core is
    py.detach(|| read_and_probe(&path))
}

/// Open the file, read its tail and its central directory, and ask the probe.
///
/// # Errors
///
/// The `io::Error` of opening, seeking or reading the file, which pyo3 raises as an `OSError`.
fn read_and_probe(path: &PathBuf) -> PyResult<bool> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let tail_size = size.min(TAIL_SIZE as u64);
    let tail_start = size - tail_size;
    let mut tail = vec![0_u8; usize::try_from(tail_size).unwrap_or(TAIL_SIZE)];
    file.seek(SeekFrom::Start(tail_start))?;
    file.read_exact(&mut tail)?;
    let Ok(located) = central_directory(&tail, tail_start) else {
        return Ok(false); // no record at the end: this is no zip container
    };
    // the offsets come from the file and are host controlled, so a directory which does not
    // fit in the file is not read and not allocated: such a container is no archive
    let end = located.offset.checked_add(located.size);
    if !end.is_some_and(|end| end <= size) {
        return Ok(false);
    }
    let Ok(length) = usize::try_from(located.size) else {
        return Ok(false);
    };
    let mut directory = vec![0_u8; length];
    file.seek(SeekFrom::Start(located.offset))?;
    file.read_exact(&mut directory)?;
    Ok(declares_manifest(&directory, located.entries).unwrap_or(false))
}
