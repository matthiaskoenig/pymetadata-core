//! The python bindings of `pymetadata-core`: the extension module `pymetadata_core._core`.

mod archive;
mod errors;

use pyo3::prelude::*;

/// The extension module of the package `pymetadata_core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("OmexError", m.py().get_type::<errors::OmexError>())?;
    m.add(
        "ArchiveBusyError",
        m.py().get_type::<errors::ArchiveBusyError>(),
    )?;
    m.add_class::<archive::Archive>()?;
    m.add_function(wrap_pyfunction!(archive::normalize_location, m)?)?;
    m.add_function(wrap_pyfunction!(archive::guess_format, m)?)?;
    m.add_function(wrap_pyfunction!(archive::lookup_format, m)?)?;
    Ok(())
}
