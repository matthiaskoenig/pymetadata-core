"""Type stubs of the extension module `pymetadata_core._core`."""

from typing import final

__version__: str
"""The version of the extension, the one of the Rust workspace."""

class OmexError(ValueError):
    """An archive, entry or location the core cannot handle.

    A subclass of `ValueError`, so code which catches `ValueError` keeps working.
    The message starts with the kind of the failure: `malformed`, `refused`,
    `unsupported` or `not found`.
    """

class ArchiveBusyError(RuntimeError):
    """An `Archive` which another thread is using.

    Inflating an entry and writing the archive run without the GIL, and an archive
    is used by one call at a time. Use one `Archive` per thread, or wait for the
    call to return.
    """

@final
class Archive:
    """A COMBINE archive which reads lazily and edits in memory.

    Opening reads the central directory and the manifest, and an entry is inflated
    when its bytes are asked for. Writing copies every entry which was not replaced
    as the compressed blob it is. The core runs without the GIL while it inflates an
    entry, copies the bytes of `set_entry`, or deflates and builds the archive, so
    another thread can work meanwhile; but the archive is in use for the duration of such a call, and a second thread which
    uses the same archive then gets an `ArchiveBusyError`, for any method but
    `from_bytes`.
    """

    def __init__(self) -> None:
        """Create an empty archive with the two entries the specification requires."""

    @staticmethod
    def from_bytes(data: bytes, password: bytes | None = None) -> Archive:
        """Open the archive in `data`.

        Args:
            data: the bytes of the `.omex` file.
            password: the password of a ZipCrypto protected archive.

        Returns:
            The archive.

        Raises:
            OmexError: if `data` is no archive, or the password is wrong.
        """

    @property
    def entries(self) -> list[tuple[str, str, bool]]:
        """The entries as `(location, format, master)`, in the order of the manifest.

        Raises:
            ArchiveBusyError: if another thread is using this archive.
        """

    def __repr__(self) -> str:
        """Return the number of entries, e.g. `Archive(2 entries)`.

        Raises:
            ArchiveBusyError: if another thread is using this archive.
        """

    def manifest_xml(self) -> str:
        """Return the `manifest.xml` the archive is written with.

        Raises:
            ArchiveBusyError: if another thread is using this archive.
        """

    def entry_bytes(self, location: str) -> bytes:
        """Return the bytes of an entry, inflated on the first ask and cached.

        Args:
            location: the location, in any spelling `normalize_location` accepts.

        Returns:
            The content of the entry.

        Raises:
            OmexError: if the entry has no bytes, which holds for `.` and for an
                unknown location, or for `./manifest.xml`, which is written from the
                entries: use `manifest_xml`.
            ArchiveBusyError: if another thread is using this archive.
        """

    def set_entry(self, location: str, format: str, master: bool, data: bytes) -> None:
        """Add an entry with its bytes, or replace the entry of the same location.

        Args:
            location: the location, normalized.
            format: the format URI, see `lookup_format` and `guess_format`.
            master: whether a tool should open the entry first.
            data: the content of the entry.

        Raises:
            OmexError: for a location outside of the archive, for `.` and for
                `./manifest.xml`; the archive is unchanged then.
            ArchiveBusyError: if another thread is using this archive.
        """

    def remove_entry(self, location: str) -> bool:
        """Remove an entry.

        Args:
            location: the location, in any spelling `normalize_location` accepts.

        Returns:
            Whether there was such an entry. `.` and `./manifest.xml` are required
            and are never removed, so they report `False`.

        Raises:
            OmexError: for a location outside of the archive, as `set_entry` and
                `entry_bytes` raise it; the archive is unchanged then.
            ArchiveBusyError: if another thread is using this archive.
        """

    def to_bytes(self, recompress: bool = False, level: int = 9) -> bytes:
        """Write the archive.

        Args:
            recompress: deflate every entry again, instead of copying every entry
                which was not replaced as the compressed blob it is.
            level: the deflate level, 0 to 9, used with `recompress`. It is checked
                either way.

        Returns:
            The bytes of the `.omex` file.

        Raises:
            OmexError: if `level` is outside of 0 to 9 (an integer wider than 128 bits
                raises `OverflowError` instead), or an entry cannot be written, e.g.
                an encrypted archive which was opened without its password.
            ArchiveBusyError: if another thread is using this archive.
        """

def normalize_location(location: str) -> str:
    """Normalize a location to a relative POSIX path with the `./` prefix.

    Args:
        location: the location to normalize.

    Returns:
        The normalized location; the archive itself is `.`.

    Raises:
        OmexError: if the location is empty, absolute or points outside of the archive.
    """

def guess_format(suffix: str, head: bytes = b"") -> str:
    """Guess the format URI of a file from its suffix and its first bytes.

    Args:
        suffix: the file suffix without the dot.
        head: the first bytes of the file, none by default; for `xml` the root
            tag tells SBML, SED-ML, CellML and COPASI apart.

    Returns:
        The format URI, or the URI of an unknown media type.
    """

def lookup_format(format_key: str) -> str:
    """Look up the format URI of a format key such as `sbml`.

    Args:
        format_key: the name of a format, without regard to case.

    Returns:
        The format URI, or the URI of an unknown media type.
    """
