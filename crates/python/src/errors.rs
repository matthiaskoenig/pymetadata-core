//! The exception of `pymetadata_core` and the mapping of the errors of the core onto it.

use pymetadata_core::OmexError as CoreError;
use pyo3::PyErr;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};

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
    MalformedError,
    OmexError,
    "Bytes which are no archive, zip container or manifest.\n\n\
     A subclass of `OmexError`, so code which catches that keeps working. The kind of the \
     failure is the class and no longer only the prefix of the message."
);

pyo3::create_exception!(
    pymetadata_core,
    RefusedError,
    OmexError,
    "Input the core refuses on purpose, e.g. a location pointing outside of the archive.\n\n\
     A subclass of `OmexError`. A wrong or missing password is the subclass `PasswordError`, \
     although its message carries the same `refused` prefix."
);

pyo3::create_exception!(
    pymetadata_core,
    UnsupportedError,
    OmexError,
    "Input which is valid and uses a feature the core does not support.\n\n\
     A subclass of `OmexError`, raised e.g. for a member under a compression method other \
     than stored or deflate."
);

pyo3::create_exception!(
    pymetadata_core,
    NotFoundError,
    OmexError,
    "An entry which does not exist.\n\n\
     A subclass of `OmexError`, so a host can tell a missing entry from a broken archive \
     without reading the message."
);

pyo3::create_exception!(
    pymetadata_core,
    PasswordError,
    RefusedError,
    "A member whose password is wrong or missing.\n\n\
     A subclass of `RefusedError`, since it is a refusal, and so of `OmexError`. It exists \
     because a host has to tell a member it may not read from one which is not there: the \
     message carries the `refused` prefix of every other refusal, and this class is what \
     separates them. Being raised proves that the member is in the container."
);

pyo3::create_exception!(
    pymetadata_core,
    ArchiveBusyError,
    PyRuntimeError,
    "An `Archive` which another thread is using.\n\n\
     Inflating an entry, setting an entry and writing run without the GIL, and an archive is used by one \
     call at a time. A `RuntimeError`, which is what pyo3 raised before for the same situation."
);

/// Turn an error of the core into the exception of its kind, keeping its message as it is.
///
/// The class is the kind of the failure, which is what the core reports in
/// [`pymetadata_core::wire::kind`] and what a host used to read off the start of the message.
/// Every class is an `OmexError`, so code which catches that, or `ValueError`, is unaffected.
///
/// The one which is no `OmexError` is [`CoreError::Io`], the sink of a write which did not take
/// the bytes: a full disk and a path which cannot be written are what `OSError` is for, and they
/// say nothing about the archive. `to_path` is the only call which can raise it.
pub fn to_py_err(error: CoreError) -> PyErr {
    let message = error.to_string();
    match error {
        CoreError::Malformed(_) => MalformedError::new_err(message),
        CoreError::Unsupported(_) => UnsupportedError::new_err(message),
        CoreError::NotFound(_) => NotFoundError::new_err(message),
        CoreError::Password(_) => PasswordError::new_err(message),
        CoreError::Refused(_) => RefusedError::new_err(message),
        CoreError::Io(_) => PyOSError::new_err(message),
    }
}
