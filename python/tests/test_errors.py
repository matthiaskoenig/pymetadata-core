"""Every way an `Archive` or a function fails, and the exception it fails with."""

import random
import threading

import pytest
from conftest import ARCHIVES, DATA

import pymetadata_core
from pymetadata_core import (
    Archive,
    ArchiveBusyError,
    OmexError,
    lookup_format,
    normalize_location,
)


def test_omex_error_is_a_value_error():
    """Code which catches `ValueError` today keeps working, as the design promises."""
    assert issubclass(OmexError, ValueError)
    assert OmexError.__module__ == "pymetadata_core"


def test_malformed_bytes_raise_omex_error():
    """Bytes which are no zip fail with the `malformed` kind, not with a crash."""
    with pytest.raises(OmexError, match="malformed") as info:
        Archive.from_bytes(b"not a zip")
    assert isinstance(info.value, ValueError)


def test_unknown_location_raises_omex_error():
    """A location the archive does not hold is a lookup failure, not malformed input."""
    archive = Archive.from_bytes(ARCHIVES[0].read_bytes())
    with pytest.raises(OmexError, match="not found"):
        archive.entry_bytes("./no/such/file.xml")


def test_the_archive_itself_has_no_bytes():
    """`.` is the archive, a manifest entry with nothing to read."""
    archive = Archive.from_bytes(ARCHIVES[0].read_bytes())
    with pytest.raises(OmexError, match="not found"):
        archive.entry_bytes(".")


def test_manifest_bytes_are_refused_and_manifest_xml_is_the_way():
    """The manifest is derived state, so its bytes come from `manifest_xml`, not the entries."""
    archive = Archive.from_bytes(ARCHIVES[0].read_bytes())
    with pytest.raises(OmexError, match="refused"):
        archive.entry_bytes("./manifest.xml")
    assert "<omexManifest" in archive.manifest_xml()


@pytest.mark.parametrize("location", ["", "/abs.xml", "C:/x.xml", "../x.xml", "a/../b"])
def test_bad_locations_are_refused(location: str):
    """A location outside of the archive is refused whichever call it reaches."""
    with pytest.raises(OmexError, match="refused"):
        normalize_location(location)
    archive = Archive()
    with pytest.raises(OmexError, match="refused"):
        archive.set_entry(location, "x", False, b"")
    # not `False`, which is what an archive without such an entry answers
    with pytest.raises(OmexError, match="refused"):
        archive.remove_entry(location)
    with pytest.raises(OmexError, match="refused"):
        archive.entry_bytes(location)


def test_manifest_cannot_be_set():
    """Writing `manifest.xml` as an entry would be overwritten by the write, so it is refused."""
    with pytest.raises(OmexError, match="refused"):
        Archive().set_entry("manifest.xml", "x", False, b"")


def test_wrong_password_raises_omex_error():
    """An encrypted manifest cannot be read without the right password."""
    data = (DATA / "zip" / "encrypted_omex.zip").read_bytes()
    with pytest.raises(OmexError):
        Archive.from_bytes(data)
    with pytest.raises(OmexError):
        Archive.from_bytes(data, password=b"wrong")


def test_right_password_opens_and_the_archive_is_written_in_the_clear():
    """The core writes no encrypted archive, so a written one needs no password."""
    data = (DATA / "zip" / "encrypted_omex.zip").read_bytes()
    archive = Archive.from_bytes(data, password=b"test1234")
    assert archive.entry_bytes("extra.txt")
    written = Archive.from_bytes(archive.to_bytes())
    assert written.entry_bytes("extra.txt") == archive.entry_bytes("extra.txt")


def test_writing_an_archive_opened_without_its_password_is_refused():
    """Copying ciphertext beside a manifest in the clear would be half encrypted."""
    data = (DATA / "zip" / "encrypted_omex.zip").read_bytes()
    with pytest.raises(OmexError):
        Archive.from_bytes(data).to_bytes()


def test_arguments_of_the_wrong_type_are_a_type_error():
    """`str` is not `bytes`: it fails loudly and is not encoded behind the caller's back."""
    with pytest.raises(TypeError):
        Archive.from_bytes("a string, not bytes")  # ty: ignore[invalid-argument-type]  deliberately the wrong type
    with pytest.raises(TypeError):
        Archive().set_entry("a.txt", "x", False, "text")  # ty: ignore[invalid-argument-type]  deliberately the wrong type


def test_an_archive_in_use_by_another_thread_says_so():
    """The price of releasing the GIL is a named error, and the archive is intact after it."""
    rng = random.Random(1)
    words = [bytes(rng.choices(b"abcdefghij", k=6)) for _ in range(2000)]
    archive = Archive()
    for k in range(2):
        data = b" ".join(rng.choices(words, k=500_000))
        archive.set_entry(f"data/{k}.txt", lookup_format("txt"), False, data)
    seen: list[ArchiveBusyError] = []
    stop = threading.Event()

    def write() -> None:
        # write until the main thread has met the busy archive, at most 50 times
        for _ in range(50):
            archive.to_bytes(recompress=True, level=9)
            if stop.is_set():
                return

    worker = threading.Thread(target=write)
    worker.start()
    try:
        while worker.is_alive() and not seen:
            try:
                archive.entries  # noqa: B018
            except ArchiveBusyError as error:
                seen.append(error)
    finally:
        stop.set()
        worker.join()
    assert seen
    assert "already in use by another thread" in str(seen[0])
    assert isinstance(seen[0], RuntimeError)
    # nothing is left borrowed: the archive works once the call returned
    assert archive.entries
    assert archive.entry_bytes("data/0.txt")


def test_the_busy_error_is_exported_and_is_a_runtime_error():
    """It is public, and a `RuntimeError` as the bare error of pyo3 was."""
    assert issubclass(ArchiveBusyError, RuntimeError)
    assert ArchiveBusyError.__module__ == "pymetadata_core"


def test_every_error_kind_has_its_own_class() -> None:
    """The kind of a failure is the class, and every class is an `OmexError`."""
    for name in (
        "MalformedError",
        "RefusedError",
        "UnsupportedError",
        "NotFoundError",
        "PasswordError",
    ):
        cls = getattr(pymetadata_core, name)
        assert issubclass(cls, OmexError), name
        assert issubclass(cls, ValueError), name
    assert issubclass(pymetadata_core.PasswordError, pymetadata_core.RefusedError)
    # the busy error says nothing about the archive, so it is deliberately outside
    assert not issubclass(pymetadata_core.ArchiveBusyError, OmexError)
    assert issubclass(pymetadata_core.ArchiveBusyError, RuntimeError)


def test_the_class_of_an_error_matches_its_message_prefix() -> None:
    """A host which read the prefix and one which catches the class agree."""
    archive = Archive.from_bytes(ARCHIVES[2].read_bytes())
    cases = [
        (
            lambda: Archive.from_bytes(b"not a zip"),
            pymetadata_core.MalformedError,
            "malformed",
        ),
        (
            lambda: archive.entry_bytes("./nothing.xml"),
            pymetadata_core.NotFoundError,
            "not found",
        ),
        (
            lambda: archive.entry_bytes("./manifest.xml"),
            pymetadata_core.RefusedError,
            "refused",
        ),
    ]
    for call, expected, prefix in cases:
        with pytest.raises(expected) as error:
            call()
        assert str(error.value).startswith(prefix), str(error.value)


def test_a_wrong_password_is_a_password_error() -> None:
    """The distinction a host needs: a password it may not pass, not a broken archive."""
    data = (DATA / "zip" / "encrypted_omex.zip").read_bytes()
    with pytest.raises(pymetadata_core.PasswordError) as missing:
        Archive.from_bytes(data)
    with pytest.raises(pymetadata_core.PasswordError) as wrong:
        Archive.from_bytes(data, password=b"wrong")
    for error in (missing, wrong):
        assert "password" in str(error.value)
        # the message keeps the prefix every other refusal carries
        assert str(error.value).startswith("refused: ")
    # a container which is no archive is a different class, which is the point
    with pytest.raises(pymetadata_core.MalformedError):
        Archive.from_bytes(b"PK\x05\x06" + bytes(18))
