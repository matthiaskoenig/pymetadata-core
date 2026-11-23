"""pymetadata-core: lazy reading and writing of COMBINE archives (OMEX)."""

from pymetadata_core._core import (
    Archive,
    ArchiveBusyError,
    MalformedError,
    NotFoundError,
    OmexError,
    PasswordError,
    RefusedError,
    UnsupportedError,
    __version__,
    guess_format,
    has_manifest,
    lookup_format,
    normalize_location,
)

__all__ = [
    "Archive",
    "ArchiveBusyError",
    "MalformedError",
    "NotFoundError",
    "OmexError",
    "PasswordError",
    "RefusedError",
    "UnsupportedError",
    "__version__",
    "guess_format",
    "has_manifest",
    "lookup_format",
    "normalize_location",
]
