"""Read archives the Rust core wrote with python's `zipfile` and with `pymetadata`.

The rust suite proves that what the core writes is read back by the core; what the
reference implementation makes of it is checked here, because the rust suite must not
depend on a python environment. The check is a manual gate and not part of `cargo test`:
the equivalent cross-check of the python layer runs in `python/tests/`.

For every archive it is given:

    1. `zipfile.ZipFile.testzip()` reports no bad member, i.e. every CRC32 holds,
    2. every member reads, and `manifest.xml` parses as a `pymetadata` manifest,
    3. `pymetadata.omex.Omex.from_omex` reads the archive and lists an entry for every
       member, with the format and the master flag the `manifest.xml` gives,
    4. the archive is compared with the one of `crates/core/tests/data/omex/` it was
       written from, where there is one: every member which was copied carries the
       compression method, the compressed size, the CRC32, the modification time and the
       external attributes of the member of the source, and every member the core
       compressed itself, which is the `manifest.xml` of a copying write and all of them
       of a recompressed write, is deflated and carries the zip epoch and mode 644.

The fourth is the property the writer exists for, which is why it is checked against the
reference implementation and not only inside the rust suite: a member nobody touched is
written as the compressed blob it was read as, and is not compressed again.

A manifest entry the archive holds no member for is reported and is no failure: an
archive of the corpus has such an entry (`iCGB21FR.omex` lists
`./iCGB21FR_memote.html`, whose member is named `iCGB21_memote.html`), and a write keeps
the manifest as it was read. Anything else exits nonzero, naming the archive and the
member.

`crates/core/tests/write.rs` writes the archives of the corpus, so run it first; the
script is run from a pymetadata checkout, so that its environment is used, like the
other scripts here:

    cd /path/to/pymetadata-core
    cargo test --locked -p pymetadata-core --test write
    cd /path/to/pymetadata
    uv run python /path/to/pymetadata-core/scripts/read_back.py \
        /path/to/pymetadata-core/target/tmp/write/*.omex
"""

import sys
import tempfile
import zipfile
from pathlib import Path

from pymetadata.omex import Manifest, ManifestEntry, Omex, normalize_location

#: the location of the archive itself, which is no member
ARCHIVE_LOCATION = "."
#: the location of the manifest, which `Omex` writes from its entries
MANIFEST_LOCATION = "./manifest.xml"
#: the corpus the written archives are compared with, where the name of a written
#: archive is the name of the archive it was written from
CORPUS = Path(__file__).resolve().parent.parent / "crates/core/tests/data/omex"
#: the prefix `crates/core/tests/write.rs` gives an archive it wrote with
#: `Archive::to_bytes_recompressed`, which copies no member at all
RECOMPRESSED = "recompressed_"
#: the zip epoch, 1980-01-01 00:00:00, which every member the core compresses itself
#: carries, since the core has no clock
EPOCH = (1980, 1, 1, 0, 0, 0)
#: the external attributes of such a member: a regular file of mode 644
REGULAR_FILE = 0o100_644 << 16


def read_manifest(xml: bytes) -> dict[str, ManifestEntry]:
    """Parse the content of a `manifest.xml` with `pymetadata`.

    Args:
        xml: Content of the `manifest.xml` of an archive.

    Returns:
        The entry of every location the manifest lists.
    """
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "manifest.xml"
        path.write_bytes(xml)
        manifest = Manifest.from_manifest(path)
    return {entry.location: entry for entry in manifest.entries}


def check(path: Path) -> list[str]:
    """Check that `zipfile` and `pymetadata` read the archive.

    Args:
        path: Path of the archive.

    Returns:
        The locations the `manifest.xml` lists and the archive holds no member for.

    Raises:
        ValueError: If a member is corrupt or does not read, if the archive has no
            `manifest.xml` which parses, or if `Omex` does not list an entry for a
            member, or lists one with another format or master flag than the
            `manifest.xml` of the archive gives.
    """
    with zipfile.ZipFile(path) as archive:
        bad = archive.testzip()
        if bad is not None:
            raise ValueError(f"'{bad}' does not match its CRC32")
        locations = {
            info.filename: normalize_location(info.filename)
            for info in archive.infolist()
            if not info.is_dir()
        }
        for name in locations:
            archive.read(name)
        manifests = [
            name
            for name, location in locations.items()
            if location == MANIFEST_LOCATION
        ]
        if not manifests:
            raise ValueError("the archive has no 'manifest.xml'")
        # the member is read under the name it has, which is `manifest.xml` or
        # `./manifest.xml`: both normalize to the location of the manifest
        listed = read_manifest(archive.read(manifests[0]))

    with Omex.from_omex(path) as omex:
        read = {entry.location: entry for entry in omex.manifest.entries}
    for name, location in sorted(locations.items()):
        if location == MANIFEST_LOCATION:
            continue
        if location not in read:
            raise ValueError(f"'{name}' is no entry of the archive pymetadata reads")
        if location not in listed:
            raise ValueError(f"'{name}' is no entry of the manifest of the archive")
        entry, expected = read[location], listed[location]
        if (entry.format, entry.master) != (expected.format, expected.master):
            raise ValueError(
                f"'{name}' is read as {entry.format}/{entry.master} and the manifest "
                f"says {expected.format}/{expected.master}"
            )
    return [
        location
        for location in listed
        if location not in (ARCHIVE_LOCATION, MANIFEST_LOCATION)
        and location not in locations.values()
    ]


def members_of(archive: zipfile.ZipFile) -> dict[str, zipfile.ZipInfo]:
    """Describe every member of an archive which is no directory, by its location.

    Args:
        archive: The open archive.

    Returns:
        What `zipfile` reads about the member of every location the archive holds.
    """
    return {
        normalize_location(info.filename): info
        for info in archive.infolist()
        if not info.is_dir()
    }


def compare(path: Path) -> str:
    """Compare an archive with the one of the corpus it was written from.

    A member whose bytes nobody changed has to be the compressed blob of the source, so
    its method, its compressed size, its CRC32, its modification time and its external
    attributes are the ones of the source. The `manifest.xml` is written from the entries
    and no copy, and a recompressed archive copies nothing at all; those members are
    deflated and carry the zip epoch and mode 644 instead.

    Args:
        path: Path of the written archive.

    Returns:
        A line about what was compared.

    Raises:
        ValueError: If a member which should be copied was compressed again, or lost its
            CRC32, its modification time or its unix mode, or if a member the core wrote
            itself does not carry what such a member carries.
    """
    recompressed = path.name.startswith(RECOMPRESSED)
    source = CORPUS / path.name.removeprefix(RECOMPRESSED)
    if not source.exists():
        return "no archive of the corpus to compare with"
    with zipfile.ZipFile(source) as archive:
        before = members_of(archive)
    with zipfile.ZipFile(path) as archive:
        after = members_of(archive)
    copied = 0
    for location, info in sorted(before.items()):
        written = after.get(location)
        if written is None:
            raise ValueError(f"'{info.filename}' of the source is no member here")
        if location == MANIFEST_LOCATION:
            continue
        if written.CRC != info.CRC:
            raise ValueError(
                f"'{info.filename}' holds the CRC32 {written.CRC:08x}, and the source "
                f"{info.CRC:08x}"
            )
        if recompressed:
            continue
        if (written.compress_type, written.compress_size) != (
            info.compress_type,
            info.compress_size,
        ):
            raise ValueError(
                f"'{info.filename}' was compressed again: method "
                f"{written.compress_type} in {written.compress_size} bytes, where the "
                f"source has method {info.compress_type} in {info.compress_size} bytes"
            )
        if written.date_time != info.date_time:
            raise ValueError(
                f"'{info.filename}' is stamped {written.date_time}, and the source "
                f"{info.date_time}"
            )
        if written.external_attr != info.external_attr:
            raise ValueError(
                f"'{info.filename}' carries the attributes 0x{written.external_attr:08x}"
                f", and the source 0x{info.external_attr:08x}"
            )
        copied += 1
    for location, written in sorted(after.items()):
        if not recompressed and location != MANIFEST_LOCATION:
            continue
        if written.compress_type != zipfile.ZIP_DEFLATED:
            raise ValueError(
                f"'{written.filename}' was written by the core and is not deflated, but "
                f"carries method {written.compress_type}"
            )
        if written.date_time != EPOCH:
            raise ValueError(
                f"'{written.filename}' was written by the core and is stamped "
                f"{written.date_time}, and not with the epoch"
            )
        if written.external_attr != REGULAR_FILE:
            raise ValueError(
                f"'{written.filename}' was written by the core and carries the "
                f"attributes 0x{written.external_attr:08x}, and not mode 644"
            )
    if recompressed:
        return f"{len(after)} members deflated again"
    return f"{copied} of {len(after)} members copied as blobs"


def main(paths: list[Path]) -> int:
    """Check every archive, and report what is wrong with it.

    Args:
        paths: Paths of the archives to read.

    Returns:
        0 if every archive reads, 1 otherwise.
    """
    if not paths:
        print(f"usage: {Path(__file__).name} ARCHIVE [ARCHIVE ...]", file=sys.stderr)
        return 1
    failed = 0
    for path in paths:
        try:
            without_member = check(path)
            compared = compare(path)
        except (ValueError, OSError, zipfile.BadZipFile) as error:
            print(f"FAIL {path.name}: {error}", file=sys.stderr)
            failed += 1
            continue
        note = ""
        if without_member:
            note = f", no member for {', '.join(sorted(without_member))}"
        print(f"ok   {path.name}: {compared}{note}")
    if failed:
        print(f"{failed} of {len(paths)} archives do not read", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main([Path(argument) for argument in sys.argv[1:]]))
