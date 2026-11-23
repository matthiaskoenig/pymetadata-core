//! The `Archive` class and the functions of the extension module.

use pymetadata_core::{Archive as CoreArchive, Entry, OmexError as CoreError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::errors::{ArchiveBusyError, to_py_err};

/// A COMBINE archive which reads lazily and edits in memory.
///
/// Opening reads the central directory and the manifest, and an entry is inflated when its bytes
/// are asked for. Writing copies every entry which was not replaced as the compressed blob it
/// is. The core runs without the GIL while it opens, inflates an entry, copies the bytes of
/// `set_entry` or writes the archive, so another thread can work meanwhile; but the archive is
/// in use for the duration of such a call (not of `from_bytes`, which has no archive yet), and a
/// second thread which uses the same archive then gets an `ArchiveBusyError`, a `RuntimeError`.
#[pyclass(module = "pymetadata_core", name = "Archive")]
pub struct Archive {
    /// The archive of the core.
    inner: CoreArchive,
}

/// The error of a thread which uses an archive while another thread has it in a call which runs
/// without the GIL, in place of the bare "Already borrowed" of pyo3.
///
/// The message names a cause which a failed `try_borrow` does not report by itself. It holds
/// because no borrow of an archive outlives a call: a guard is taken and dropped under the GIL,
/// and the only place where it is held while another thread can run is inside a `detach`: the
/// inflate, the copy of `set_entry` and the write. So a borrow which fails is a call of another thread in one of those.
fn busy<E>(_: E) -> PyErr {
    ArchiveBusyError::new_err(
        "this Archive is already in use by another thread, which is inflating an entry, \
         setting an entry or writing the archive; use one Archive per thread, or wait for that call to return",
    )
}

/// The deflate level, 0 to 9, from the argument of `to_bytes`.
///
/// The argument is a 128 bit integer, so that every level a caller can reasonably write, the
/// negative ones included, reaches here and is an `OmexError` when it is outside of the range,
/// where a `u32` would have raised pyo3's `OverflowError` for a negative one.
fn level_of(level: i128) -> PyResult<u32> {
    u32::try_from(level)
        .ok()
        .filter(|level| *level <= 9)
        .ok_or_else(|| {
            to_py_err(CoreError::Refused(format!(
                "the compression level {level} is outside of 0 to 9"
            )))
        })
}

#[pymethods]
impl Archive {
    /// An empty archive, holding the two entries the specification requires.
    #[new]
    fn new() -> Self {
        Self {
            inner: CoreArchive::new(),
        }
    }

    /// Open the archive in `data`, with `password` for a ZipCrypto protected one.
    #[staticmethod]
    #[pyo3(signature = (data, password=None))]
    fn from_bytes(py: Python<'_>, data: &[u8], password: Option<&[u8]>) -> PyResult<Self> {
        let inner = py
            .detach(|| match password {
                Some(password) => CoreArchive::from_bytes_with_password(data, password),
                None => CoreArchive::from_bytes(data),
            })
            .map_err(to_py_err)?;
        Ok(Self { inner })
    }

    /// The entries as `(location, format, master)`, in the order of the manifest.
    #[getter]
    fn entries(slf: &Bound<'_, Self>) -> PyResult<Vec<(String, String, bool)>> {
        let this = slf.try_borrow().map_err(busy)?;
        Ok(this
            .inner
            .manifest()
            .entries()
            .iter()
            .map(|entry| (entry.location.clone(), entry.format.clone(), entry.master))
            .collect())
    }

    /// The entries the `manifest.xml` of the source container declared, as
    /// `(location, format, master)` in document order, without the entries invented for
    /// unlisted members. A location declared twice is listed twice. Fixed on opening, so
    /// `set_entry` and `remove_entry` do not change it; empty for an archive built from
    /// nothing.
    #[getter]
    fn declared_entries(slf: &Bound<'_, Self>) -> PyResult<Vec<(String, String, bool)>> {
        let this = slf.try_borrow().map_err(busy)?;
        Ok(this
            .inner
            .declared_entries()
            .iter()
            .map(|entry| (entry.location.clone(), entry.format.clone(), entry.master))
            .collect())
    }

    /// The members of the source container as
    /// `(location, compressed size, uncompressed size, method)`, in zip order.
    ///
    /// Nothing is inflated. It is empty for an archive built from nothing and does not change
    /// with `set_entry`.
    #[getter]
    fn members(slf: &Bound<'_, Self>) -> PyResult<Vec<(String, u64, u64, u16)>> {
        let this = slf.try_borrow().map_err(busy)?;
        Ok(this
            .inner
            .members()
            .into_iter()
            .map(|m| (m.location, m.compressed_size, m.uncompressed_size, m.method))
            .collect())
    }

    /// Whether the location has bytes, in the source container or from `set_entry`.
    fn has_bytes(slf: &Bound<'_, Self>, location: &str) -> PyResult<bool> {
        let this = slf.try_borrow().map_err(busy)?;
        this.inner.has_bytes(location).map_err(to_py_err)
    }

    /// The `manifest.xml` the archive is written with.
    fn manifest_xml(slf: &Bound<'_, Self>) -> PyResult<String> {
        let this = slf.try_borrow().map_err(busy)?;
        Ok(this.inner.manifest().to_xml())
    }

    /// The bytes of an entry, inflated on the first ask and cached.
    fn entry_bytes<'py>(slf: &Bound<'py, Self>, location: &str) -> PyResult<Bound<'py, PyBytes>> {
        let py = slf.py();
        let mut this = slf.try_borrow_mut().map_err(busy)?;
        let inner = &mut this.inner;
        // inflate without the GIL; the bytes stay in the archive, so the second call below
        // answers from the cache and inflates nothing
        py.detach(|| inner.entry_bytes(location).map(|_| ()))
            .map_err(to_py_err)?;
        let bytes = inner.entry_bytes(location).map_err(to_py_err)?;
        Ok(PyBytes::new(py, bytes))
    }

    /// Add an entry with its bytes, or replace the entry of the same location.
    fn set_entry(
        slf: &Bound<'_, Self>,
        location: &str,
        format: &str,
        master: bool,
        data: &[u8],
    ) -> PyResult<()> {
        let py = slf.py();
        let mut this = slf.try_borrow_mut().map_err(busy)?;
        let inner = &mut this.inner;
        // the copy of the bytes of a large entry runs without the GIL as well
        py.detach(|| {
            let entry = Entry::new(location, format, master)?;
            inner.set_entry(entry, data.to_vec())
        })
        .map_err(to_py_err)
    }

    /// Change the format and the master flag of an entry, without touching its bytes.
    ///
    /// `set_entry` takes the bytes of the entry, so changing only its metadata through it
    /// inflates the member and deflates it again. This changes the manifest entry alone, so
    /// the member stays the compressed blob `to_bytes` copies. The entry keeps its position.
    ///
    /// A location the manifest does not list is an `OmexError`, as are `.` and
    /// `./manifest.xml`, whose entries the specification prescribes.
    fn set_entry_format(
        slf: &Bound<'_, Self>,
        location: &str,
        format: &str,
        master: bool,
    ) -> PyResult<()> {
        let mut this = slf.try_borrow_mut().map_err(busy)?;
        this.inner
            .set_entry_format(location, format, master)
            .map_err(to_py_err)
    }

    /// Remove an entry, and say whether there was one.
    ///
    /// A location outside of the archive is an `OmexError`, as it is for `set_entry` and
    /// `entry_bytes`, and not the `False` of an archive which holds no such entry.
    fn remove_entry(slf: &Bound<'_, Self>, location: &str) -> PyResult<bool> {
        let mut this = slf.try_borrow_mut().map_err(busy)?;
        let removed = this.inner.remove_entry(location).map_err(to_py_err)?;
        Ok(removed.is_some())
    }

    /// Write the archive.
    ///
    /// Without `recompress`, every entry which was not replaced is copied as the compressed
    /// blob it is. With it, every entry is deflated again at `level` (0 to 9); a level outside of
    /// that range is an `OmexError`.
    #[pyo3(name = "to_bytes", signature = (recompress=false, level=9))]
    fn write<'py>(
        slf: &Bound<'py, Self>,
        recompress: bool,
        level: i128,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let py = slf.py();
        let level = level_of(level)?;
        let mut this = slf.try_borrow_mut().map_err(busy)?;
        let inner = &mut this.inner;
        let bytes = py
            .detach(|| {
                if recompress {
                    inner.to_bytes_recompressed(level)
                } else {
                    inner.to_bytes()
                }
            })
            .map_err(to_py_err)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Show how many entries there are.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let this = slf.try_borrow().map_err(busy)?;
        Ok(format!(
            "Archive({} entries)",
            this.inner.manifest().entries().len()
        ))
    }
}

/// Normalize a location to a relative POSIX path with the `./` prefix.
#[pyfunction]
pub fn normalize_location(location: &str) -> PyResult<String> {
    pymetadata_core::normalize_location(location).map_err(to_py_err)
}

/// Guess the format URI from the suffix, without the dot, and the first bytes of a file.
#[pyfunction]
// the default is spelled out for `inspect.signature`, which shows `Ellipsis` for a slice
#[pyo3(signature = (suffix, head=&b""[..]), text_signature = "(suffix, head=b'')")]
pub fn guess_format(suffix: &str, head: &[u8]) -> &'static str {
    pymetadata_core::guess_format(suffix, head)
}

/// Look up the format URI of a format key such as `sbml`; an unknown key is the unknown URI.
#[pyfunction]
pub fn lookup_format(format_key: &str) -> &'static str {
    pymetadata_core::lookup_format(format_key)
}
