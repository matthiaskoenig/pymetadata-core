"""The behavior of `Archive` and the format functions, checked against `zipfile`."""

import inspect
import io
import struct
import zipfile
from pathlib import Path

import pytest
from conftest import ARCHIVES

from pymetadata_core import (
    Archive,
    OmexError,
    guess_format,
    lookup_format,
    normalize_location,
)

MANIFEST = "./manifest.xml"


def test_there_are_four_fixtures():
    """The corpus is the four archives of the core, so that no test runs on fewer."""
    assert [path.stem for path in ARCHIVES] == [
        "BIOMD0000000001",
        "CombineArchiveShowCase",
        "CompModels",
        "iCGB21FR",
    ]


def test_opens_every_fixture(archive_path: Path):
    """Opening reads the manifest only, and it always lists `.` and the manifest itself."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    locations = [location for location, _, _ in archive.entries]
    assert locations
    assert "." in locations
    assert MANIFEST in locations
    assert archive.manifest_xml().startswith("<?xml")


def test_entry_bytes_matches_zipfile(archive_path: Path):
    """The inflated bytes are the ones the zip standard library reads, per member."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    with zipfile.ZipFile(archive_path) as zf:
        for info in zf.infolist():
            if info.is_dir():
                continue
            location = "./" + info.filename.removeprefix("./")
            if location == MANIFEST:
                continue
            assert archive.entry_bytes(location) == zf.read(info.filename), location


def test_entry_bytes_accepts_any_spelling_of_a_location():
    """A caller need not know the `./` form: the core normalizes at its edge."""
    archive = Archive.from_bytes(ARCHIVES[0].read_bytes())
    location = next(loc for loc, _, _ in archive.entries if loc not in (".", MANIFEST))
    expected = archive.entry_bytes(location)
    assert archive.entry_bytes(location[2:]) == expected
    assert archive.entry_bytes(location.replace("/", "\\")) == expected


def test_entries_are_tuples_of_location_format_master(archive_path: Path):
    """The shape of `entries` is the contract of the stub, `master` is a real bool."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    for entry in archive.entries:
        location, format, master = entry
        assert isinstance(entry, tuple)
        assert isinstance(location, str)
        assert isinstance(format, str)
        assert isinstance(master, bool)


def test_set_entry_then_to_bytes_is_readable_by_zipfile(archive_path: Path):
    """A written archive is a valid zip with the new member and a manifest."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    archive.set_entry("new/notes.txt", lookup_format("txt"), True, b"hello")
    data = archive.to_bytes()
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        assert zf.testzip() is None
        assert zf.read("new/notes.txt") == b"hello"
        assert "manifest.xml" in zf.namelist()
    assert ("./new/notes.txt", lookup_format("txt"), True) in (
        Archive.from_bytes(data).entries
    )


def test_set_entry_replaces_and_remove_entry_reports(archive_path: Path):
    """Setting a location twice keeps one entry, and removing says whether it was there."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    archive.set_entry("a.txt", lookup_format("txt"), False, b"one")
    archive.set_entry("./a.txt", lookup_format("txt"), False, b"two")
    assert archive.entry_bytes("a.txt") == b"two"
    assert [loc for loc, _, _ in archive.entries].count("./a.txt") == 1
    assert archive.remove_entry("a.txt") is True
    assert archive.remove_entry("a.txt") is False


def test_an_archive_written_without_a_change_has_the_content_it_was_read_with(
    archive_path: Path,
):
    """With no `set_entry` at all, every member comes back with its content."""
    source = archive_path.read_bytes()
    written = Archive.from_bytes(source).to_bytes()
    with (
        zipfile.ZipFile(io.BytesIO(source)) as before,
        zipfile.ZipFile(io.BytesIO(written)) as after,
    ):
        for info in before.infolist():
            if info.is_dir() or info.filename == "manifest.xml":
                continue
            assert after.read(info.filename) == before.read(info.filename)


def raw_member(data: bytes, info: zipfile.ZipInfo) -> bytes:
    """The compressed bytes of a member, as they are in the zip."""
    start = info.header_offset
    name_length, extra_length = struct.unpack("<HH", data[start + 26 : start + 30])
    begin = start + 30 + name_length + extra_length
    return data[begin : begin + info.compress_size]


def test_to_bytes_copies_untouched_members_as_the_blobs_they_are(archive_path: Path):
    """The untouched members are copied, not deflated again, which is the point of the core.

    Deflating the same bytes with another implementation would give other compressed bytes,
    so the same compressed bytes, CRC and size prove the member was copied.
    """
    source = archive_path.read_bytes()
    archive = Archive.from_bytes(source)
    archive.set_entry("one.txt", lookup_format("txt"), False, b"changed")
    written = archive.to_bytes()
    with (
        zipfile.ZipFile(io.BytesIO(source)) as before,
        zipfile.ZipFile(io.BytesIO(written)) as after,
    ):
        assert after.testzip() is None
        copied = {info.filename: info for info in after.infolist()}
        checked = 0
        for info in before.infolist():
            if info.is_dir() or info.filename == "manifest.xml":
                continue
            other = copied[info.filename]
            assert other.CRC == info.CRC
            assert other.compress_type == info.compress_type
            assert other.compress_size == info.compress_size
            # the compressed bytes themselves are the same ones, not a new deflate of them
            assert raw_member(written, other) == raw_member(source, info)
            checked += 1
        assert checked


def test_recompress_deflates_every_member_again(archive_path: Path):
    """`recompress=True` is the explicit way to rewrite every member, and it loses nothing."""
    source = archive_path.read_bytes()
    recompressed = Archive.from_bytes(source).to_bytes(recompress=True, level=1)
    with (
        zipfile.ZipFile(io.BytesIO(source)) as before,
        zipfile.ZipFile(io.BytesIO(recompressed)) as after,
    ):
        assert after.testzip() is None
        for info in before.infolist():
            if info.is_dir() or info.filename == "manifest.xml":
                continue
            assert after.read(info.filename) == before.read(info.filename)
            assert after.getinfo(info.filename).CRC == info.CRC


def test_new_archive_is_empty_with_the_two_default_entries():
    """A new archive holds exactly the two entries the specification requires."""
    archive = Archive()
    assert [location for location, _, _ in archive.entries] == [".", MANIFEST]
    assert Archive.from_bytes(archive.to_bytes()).entries == archive.entries


@pytest.mark.parametrize("level", [-1, -(2**40), 10, 100, 2**70])
@pytest.mark.parametrize("recompress", [True, False])
def test_a_level_outside_of_0_to_9_is_an_omex_error(level: int, recompress: bool):
    """One error type for every bad level, a negative one is no `OverflowError`."""
    with pytest.raises(OmexError, match="compression level"):
        Archive().to_bytes(recompress=recompress, level=level)


@pytest.mark.parametrize("level", [0, 1, 9])
def test_every_level_of_0_to_9_writes_a_readable_archive(level: int):
    """0 and 9 are the lowest and highest levels deflate knows, so neither may be refused."""
    data = Archive().to_bytes(recompress=True, level=level)
    assert Archive.from_bytes(data).entries == Archive().entries


def test_a_level_which_is_no_integer_is_a_type_error():
    """A wrong type is a `TypeError`, as for the other arguments, not an `OmexError`."""
    with pytest.raises(TypeError):
        Archive().to_bytes(level="9")  # ty: ignore[invalid-argument-type]  deliberately the wrong type


def test_normalize_location_cases():
    """The documented normalizations: prefix, backslashes, empty segments."""
    assert normalize_location("models/model.xml") == "./models/model.xml"
    assert normalize_location(".\\models\\model.xml") == "./models/model.xml"
    assert normalize_location("./manifest.xml") == "./manifest.xml"
    assert normalize_location(".") == "."
    assert normalize_location("./models//model.xml") == "./models/model.xml"


def test_guess_format_reads_the_root_tag():
    """For `xml` the root tag decides, for any other suffix the suffix does."""
    head = b'<?xml version="1.0"?>\n<sbml xmlns="http://www.sbml.org/sbml/level3">'
    assert guess_format("xml", head) == lookup_format("sbml")
    assert guess_format("xml") == lookup_format("xml")
    assert guess_format("csv") == lookup_format("csv")
    assert guess_format("XML", head) == lookup_format("sbml")


def test_lookup_format_unknown_key_is_the_unknown_uri():
    """An unknown key is not an error, it is the unknown media type."""
    assert lookup_format("no-such-format").endswith("application/x.unknown")


def test_guess_format_head_defaults_to_empty():
    """`head` is optional and empty by default, so a suffix alone is enough."""
    assert guess_format("xml") == guess_format("xml", b"")
    assert inspect.signature(guess_format).parameters["head"].default == b""


def test_repr_names_the_number_of_entries():
    """A bare `<Archive object at 0x...>` says nothing in a traceback, the count does."""
    assert repr(Archive()) == "Archive(2 entries)"
