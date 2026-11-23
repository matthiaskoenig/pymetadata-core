"""Cross-checks against the released `pymetadata`, the behavior to stay faithful to."""

import re
import tempfile
import zipfile
from pathlib import Path
from xml.etree import ElementTree

import pytest
from conftest import ARCHIVES
from pymetadata.omex import Omex
from pymetadata.omex import normalize_location as pymetadata_normalize_location

from pymetadata_core import (
    Archive,
    OmexError,
    guess_format,
    lookup_format,
    normalize_location,
)


def manifest_of(path: Path) -> list[tuple[str, str, bool]]:
    """The manifest `pymetadata` reads from an archive file."""
    with Omex.from_omex(path) as omex:
        return [(e.location, e.format, e.master) for e in omex.manifest.entries]


def dangling(archive: Archive) -> set[str]:
    """The entries the manifest lists and the zip holds no member for."""
    found = set()
    for location, _, _ in archive.entries:
        if location in (".", "./manifest.xml"):
            continue
        try:
            archive.entry_bytes(location)
        except OmexError:
            found.add(location)
    return found


def assert_same_manifest(
    archive: Archive, theirs: list[tuple[str, str, bool]], order: list[str]
) -> None:
    """Assert `pymetadata` reads the manifest of the core, up to two known differences.

    The core keeps an entry the manifest lists and the zip has no member for (a dangling
    entry), `pymetadata` drops it, since it builds its manifest from the extracted files.
    The core keeps the files the manifest does not list in the order of the zip,
    `pymetadata` sorts them by location. `pymetadata` also always gives `.` and `./manifest.xml` the format it
    has by default, where the core keeps the one the file names, which differs for an
    archive that spells it with `https`. Everything else is equal, entry by entry.

    Args:
        archive: the archive of the core.
        theirs: the entries `pymetadata` read.
        order: the order the locations which both hold must have.
    """
    ours = {location: (fmt, master) for location, fmt, master in archive.entries}
    others = {location: (fmt, master) for location, fmt, master in theirs}
    assert set(ours) - set(others) == dangling(archive)
    assert set(others) <= set(ours)
    for location, value in others.items():
        if location in (".", "./manifest.xml"):
            assert ours[location][1] == value[1]
        else:
            assert ours[location] == value, location
    assert [loc for loc, _, _ in theirs if loc in order] == [
        loc for loc in order if loc in others
    ]


def listed_in_manifest(path: Path) -> list[str]:
    """The locations the manifest of the archive file lists, in its order."""
    with zipfile.ZipFile(path) as zf:
        xml = zf.read("manifest.xml").decode()
    return [
        pymetadata_normalize_location(location)
        for location in re.findall(r'location="([^"]*)"', xml)
    ]


def test_pymetadata_reads_the_same_manifest_as_the_core(archive_path: Path):
    """The two readers agree on every entry, up to the documented differences."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    # both put the two entries the specification requires first
    listed = [".", "./manifest.xml", *listed_in_manifest(archive_path)]
    listed = list(dict.fromkeys(listed))
    theirs = manifest_of(archive_path)
    assert_same_manifest(archive, theirs, listed)
    # the listed entries come first, then the others by location
    locations = [loc for loc, _, _ in theirs]
    first = [loc for loc in listed if loc in locations]
    assert locations[: len(first)] == first
    assert locations[len(first) :] == sorted(locations[len(first) :])


def test_roundtrip_through_pymetadata(archive_path: Path, tmp_path: Path):
    """What the core writes, `pymetadata` reads, with the entry that was added."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    archive.set_entry("added/data.csv", lookup_format("csv"), False, b"a,b\n1,2\n")
    written = tmp_path / "written.omex"
    written.write_bytes(archive.to_bytes())
    order = [location for location, _, _ in archive.entries]
    assert_same_manifest(archive, manifest_of(written), order)
    assert ("./added/data.csv", lookup_format("csv"), False) in manifest_of(written)
    with Omex.from_omex(written) as omex:
        assert omex.get_path("./added/data.csv").read_bytes() == b"a,b\n1,2\n"


def test_unchanged_archive_has_the_manifest_of_pymetadata(
    archive_path: Path, tmp_path: Path
):
    """A copy-only write loses nothing `pymetadata` can see in the manifest."""
    archive = Archive.from_bytes(archive_path.read_bytes())
    written = tmp_path / "written.omex"
    written.write_bytes(archive.to_bytes())
    order = [location for location, _, _ in archive.entries]
    assert_same_manifest(archive, manifest_of(written), order)


def test_the_core_keeps_a_dangling_entry_which_pymetadata_drops():
    """Difference 1: the core reports what the manifest says, even for a missing member."""
    path = next(p for p in ARCHIVES if p.stem == "iCGB21FR")
    archive = Archive.from_bytes(path.read_bytes())
    assert dangling(archive) == {"./iCGB21FR_memote.html"}
    assert "./iCGB21FR_memote.html" not in [loc for loc, _, _ in manifest_of(path)]


@pytest.mark.parametrize(
    "location",
    [
        "models/model.xml",
        ".\\models\\model.xml",
        "./manifest.xml",
        ".",
        "./models//model.xml",
        "a/./b",
        "./a/b/",
        "",
        "/model.xml",
        "C:/model.xml",
        "c:model.xml",
        "../model.xml",
        "a/../b",
        "..\\model.xml",
    ],
)
def test_normalize_location_matches_pymetadata(location: str):
    """The same inputs are accepted and refused, with the same result."""
    try:
        expected = pymetadata_normalize_location(location)
    except ValueError:
        with pytest.raises(OmexError):
            normalize_location(location)
    else:
        assert normalize_location(location) == expected


def test_lookup_format_matches_pymetadata():
    """The format table is the one of `EntryFormat`, including the case insensitive key."""
    for key in ("sbml", "SBML", "csv", "sedml", "omex", "no-such-key", "txt"):
        assert lookup_format(key) == Omex.lookup_format(key), key


@pytest.mark.parametrize(
    ("name", "head"),
    [
        ("a.xml", b'<?xml version="1.0"?><sbml level="3">'),
        ("a.xml", b"<sedML xmlns='x'>"),
        ("a.xml", b"<cell xmlns='x'>"),
        ("a.xml", b"plain"),
        ("a.csv", b"x"),
        ("a.XML", b"<sbml>"),
        ("a.unknownsuffix", b""),
    ],
)
def test_guess_format_matches_pymetadata(name: str, head: bytes):
    """The root tag of an `xml` file is looked at in the same bytes, the suffix otherwise."""
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / name
        path.write_bytes(head)
        expected = Omex.guess_format(path)
    assert guess_format(Path(name).suffix[1:], head) == expected


DEFAULT_FORMATS = {
    ".": "http://identifiers.org/combine.specifications/omex",
    "./manifest.xml": "http://identifiers.org/combine.specifications/omex-manifest",
}
"""What the core gives `.` and the manifest when the file does not list them."""


def declared_formats(path: Path) -> dict[str, str]:
    """The format the manifest text of an archive declares per location."""
    with zipfile.ZipFile(path) as zf:
        root = ElementTree.fromstring(zf.read("manifest.xml"))
    return {
        pymetadata_normalize_location(element.get("location", "")): element.get(
            "format", ""
        )
        for element in root.iter()
        if element.tag.rpartition("}")[2] == "content"
    }


def test_the_core_keeps_the_format_of_the_two_core_entries_of_the_file(
    archive_path: Path,
):
    """Difference 3, pinned for every archive: the declared format, or else the default."""
    declared = declared_formats(archive_path)
    ours = {
        loc: fmt
        for loc, fmt, _ in Archive.from_bytes(archive_path.read_bytes()).entries
    }
    for location, default in DEFAULT_FORMATS.items():
        assert ours[location] == declared.get(location, default), location


def test_pymetadata_always_gives_the_two_core_entries_its_own_format(
    archive_path: Path,
):
    """Difference 3, the other side: `pymetadata` ignores what the file declares."""
    theirs = {loc: fmt for loc, fmt, _ in manifest_of(archive_path)}
    for location, default in DEFAULT_FORMATS.items():
        assert theirs[location] == default, location


def test_the_core_keeps_the_https_format_a_file_declares():
    """Difference 3 where it shows: iCGB21FR spells `.` with `https`."""
    path = next(p for p in ARCHIVES if p.stem == "iCGB21FR")
    ours = {loc: fmt for loc, fmt, _ in Archive.from_bytes(path.read_bytes()).entries}
    assert ours["."] == "https://identifiers.org/combine.specifications/omex"


def test_the_core_keeps_unlisted_files_in_the_order_of_the_zip():
    """Difference 2, the core's side: unlisted members follow the zip, not the sort."""
    path = next(p for p in ARCHIVES if p.stem == "iCGB21FR")
    listed = set(declared_formats(path))
    with zipfile.ZipFile(path) as zf:
        in_zip = [
            "./" + info.filename.removeprefix("./")
            for info in zf.infolist()
            if not info.is_dir()
        ]
    unlisted_in_zip = [loc for loc in in_zip if loc not in listed]
    archive = Archive.from_bytes(path.read_bytes())
    ours = [loc for loc, _, _ in archive.entries if loc not in listed]
    assert ours == unlisted_in_zip
    # the difference is real for this archive: sorting would reorder them
    assert ours != sorted(ours)
