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


def test_members_are_tuples_whose_first_element_is_the_location():
    """The members of the container are `(location, compressed, uncompressed, method)`."""
    source = ARCHIVES[-1].read_bytes()
    archive = Archive.from_bytes(source)
    members = archive.members
    assert isinstance(members, list)
    assert all(isinstance(member, tuple) and len(member) == 4 for member in members)
    with zipfile.ZipFile(io.BytesIO(source)) as zf:
        infos = [info for info in zf.infolist() if not info.is_dir()]
    assert [m[0] for m in members] == [f"./{info.filename}" for info in infos]
    assert [m[1:] for m in members] == [
        (info.compress_size, info.file_size, info.compress_type) for info in infos
    ]


def test_members_are_empty_for_a_new_archive_and_ignore_set_entry():
    """`members` describes the source container, `has_bytes` sees the staged content."""
    archive = Archive()
    assert archive.members == []
    archive.set_entry("a.txt", lookup_format("txt"), False, b"x")
    assert archive.members == []
    assert archive.has_bytes("a.txt")


def test_has_bytes_is_false_for_a_listed_entry_without_a_member():
    """`./iCGB21FR_memote.html` is listed by the manifest and no member provides it."""
    archive = Archive.from_bytes(ARCHIVES[-1].read_bytes())
    assert "./iCGB21FR_memote.html" in [e[0] for e in archive.entries]
    assert archive.has_bytes("./iCGB21FR_memote.html") is False
    assert archive.has_bytes("iCGB21FR.xml") is True
    assert archive.has_bytes(".") is False
    assert archive.has_bytes(MANIFEST) is False


def test_has_bytes_refuses_a_location_outside_of_the_archive():
    """A location which cannot be normalized is an `OmexError`, as for `entry_bytes`."""
    with pytest.raises(OmexError):
        Archive().has_bytes("../x")


def test_declared_entries_are_the_manifest_without_the_invented_ones():
    """The declared entries are a strict subset of `entries`, without the junk members."""
    archive = Archive.from_bytes(ARCHIVES[-1].read_bytes())
    declared = [location for location, _, _ in archive.declared_entries]
    entries = [location for location, _, _ in archive.entries]
    assert set(declared) < set(entries)
    assert "./iCGB21FR_memote.html" in declared
    assert not any(
        "__MACOSX" in location or ".DS_Store" in location for location in declared
    )
    assert all(len(entry) == 3 for entry in archive.declared_entries)
    archive.set_entry("new.txt", lookup_format("txt"), False, b"x")
    assert [e[0] for e in archive.declared_entries] == declared
    assert Archive().declared_entries == []


def test_the_stub_declares_members_and_has_bytes():
    """The stub names what the runtime has."""
    stub = (Path(__file__).parent.parent / "pymetadata_core" / "_core.pyi").read_text()
    for name in ("members", "declared_entries"):
        assert type(getattr(Archive, name)).__name__ == "getset_descriptor"
        assert f"@property\n    def {name}(" in stub
    assert list(inspect.signature(Archive.has_bytes).parameters) == ["self", "location"]
    assert "def has_bytes(self, location: str) -> bool:" in stub


def test_set_entry_format_changes_the_manifest_and_copies_the_member(
    archive_path: Path,
) -> None:
    """A metadata only change does not recompress the member it describes."""
    data = archive_path.read_bytes()
    before = {
        member.filename: (member.compress_type, member.compress_size, member.CRC)
        for member in zipfile.ZipFile(io.BytesIO(data)).infolist()
        if not member.is_dir()
    }
    archive = Archive.from_bytes(data)
    target = next(
        location
        for location, _, _ in archive.entries
        if location.endswith(".xml") and location != MANIFEST
    )
    archive.set_entry_format(target, lookup_format("txt"), True)
    assert (target, lookup_format("txt"), True) in archive.entries
    written = zipfile.ZipFile(io.BytesIO(archive.to_bytes()))
    for member in written.infolist():
        name = member.filename
        if name == "manifest.xml" or name not in before:
            continue
        assert (member.compress_type, member.compress_size, member.CRC) == before[
            name
        ], f"{name} was rewritten"
    assert f'format="{lookup_format("txt")}"' in archive.manifest_xml()


def test_set_entry_format_refuses_what_it_cannot_change() -> None:
    """An unknown location is refused, as are the two prescribed entries."""
    archive = Archive.from_bytes(ARCHIVES[2].read_bytes())
    for location in (".", MANIFEST, "manifest.xml", "./nothing.xml"):
        with pytest.raises(OmexError):
            archive.set_entry_format(location, lookup_format("txt"), False)
    assert "./nothing.xml" not in [location for location, _, _ in archive.entries]


def test_set_entry_format_keeps_the_order_of_the_entries() -> None:
    """The entry stays where it was in the manifest."""
    archive = Archive.from_bytes(ARCHIVES[2].read_bytes())
    before = [location for location, _, _ in archive.entries]
    archive.set_entry_format(before[-1], lookup_format("txt"), False)
    assert [location for location, _, _ in archive.entries] == before


def test_reorder_entries_sets_the_order_of_the_written_archive() -> None:
    """The order a host sets is the order of the written manifest and its members."""
    archive = Archive.from_bytes(ARCHIVES[3].read_bytes())
    wanted = sorted(
        location
        for location, _, _ in archive.entries
        if location not in (".", MANIFEST)
    )
    assert wanted != [
        location
        for location, _, _ in archive.entries
        if location not in (".", MANIFEST)
    ], "the fixture is worth the test only if the orders differ"
    archive.reorder_entries(wanted)
    written = Archive.from_bytes(archive.to_bytes())
    assert [
        location
        for location, _, _ in written.entries
        if location not in (".", MANIFEST)
    ] == wanted
    names = [
        name
        for name in zipfile.ZipFile(io.BytesIO(archive.to_bytes())).namelist()
        if name != "manifest.xml"
    ]
    # the fixture declares one entry no member provides, which stays a manifest entry
    # without becoming a member, so the members are the entries which have bytes
    assert names == [
        location.removeprefix("./")
        for location in wanted
        if archive.has_bytes(location)
    ]
    assert not archive.has_bytes("./iCGB21FR_memote.html")


def test_reorder_entries_refuses_an_order_it_cannot_apply() -> None:
    """An unknown location, a repeated one and one outside of the archive are refused."""
    archive = Archive.from_bytes(ARCHIVES[2].read_bytes())
    before = [location for location, _, _ in archive.entries]
    for order in (["./nothing.xml"], [before[2], before[2]], ["../evil.txt"]):
        with pytest.raises(OmexError):
            archive.reorder_entries(order)
    assert [location for location, _, _ in archive.entries] == before


def test_head_is_the_prefix_of_the_entry(archive_path: Path) -> None:
    """A head is what `entry_bytes` starts with, for every readable entry."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    for location, _, _ in archive.entries:
        if location in (".", MANIFEST) or not archive.has_bytes(location):
            continue
        try:
            head = archive.head(location, 256)
        except OmexError:
            continue  # a member this core cannot read has no head either
        assert head == archive.entry_bytes(location)[:256], location
        assert len(head) <= 256


def test_head_does_not_cache_and_serves_what_is_at_hand() -> None:
    """A head leaves the entry unread, and uses the bytes of one which was read."""
    archive = Archive.from_bytes(ARCHIVES[2].read_bytes())
    archive.set_entry("new.txt", lookup_format("txt"), False, b"0123456789")
    assert archive.head("./new.txt", 4) == b"0123"
    assert archive.head("./new.txt", 99) == b"0123456789"
    location = next(
        name
        for name, _, _ in archive.entries
        if name.endswith(".xml") and name != MANIFEST
    )
    assert archive.head(location, 0) == b""
    whole = archive.entry_bytes(location)
    assert archive.head(location, 8) == whole[:8]


def test_head_refuses_what_it_cannot_read() -> None:
    """The manifest, an unknown location and a negative limit are errors."""
    archive = Archive.from_bytes(ARCHIVES[2].read_bytes())
    for location, limit in ((MANIFEST, 8), (".", 8), ("./nothing.xml", 8), ("../x", 8)):
        with pytest.raises(OmexError):
            archive.head(location, limit)
    location = next(
        name
        for name, _, _ in archive.entries
        if name.endswith(".xml") and name != MANIFEST
    )
    with pytest.raises(OmexError, match="negative"):
        archive.head(location, -1)


def test_head_classifies_an_xml_member_without_inflating_it() -> None:
    """The case this exists for: the format of an unlisted member from its first bytes."""
    archive = Archive.from_bytes(ARCHIVES[3].read_bytes())
    unlisted = [
        location
        for location, _, _ in archive.entries
        if location.endswith(".xml")
        and location != MANIFEST
        and archive.has_bytes(location)
    ]
    assert unlisted, "the fixture holds xml members"
    for location in unlisted:
        head = archive.head(location, 256)
        assert guess_format("xml", head) == guess_format(
            "xml", archive.entry_bytes(location)[:256]
        ), location
