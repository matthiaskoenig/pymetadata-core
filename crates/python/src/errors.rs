//! The exception of `pymetadata_core` and the mapping of the errors of the core onto it.

use pymetadata_core::OmexError as CoreError;
use pyo3::PyErr;
use pyo3::exceptions::{PyRuntimeError, PyValueError};

pyo3::create_exception!(
    pymetadata_core,
    OmexError,
    PyValueError,
    "An archive, entry or location the core cannot handle.\n\n\
     A subclass of `ValueError`, so code which catches `ValueError` keeps working. The message \
     starts with the kind of the failure: `malformed`, `refused`, `unsupported` or `not found`."
);

pyo3::create_exception!(
    pymetadata_core,
    ArchiveBusyError,
    PyRuntimeError,
    "An `Archive` which another thread is using.\n\n\
     Inflating an entry, setting an entry and writing run without the GIL, and an archive is used by one \
     call at a time. A `RuntimeError`, which is what pyo3 raised before for the same situation."
);

/// Turn an error of the core into the exception, keeping the message of the core as it is.
pub fn to_py_err(error: CoreError) -> PyErr {
    OmexError::new_err(error.to_string())
}
