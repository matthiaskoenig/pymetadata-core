"""Build the zip edge case fixtures of the core with python's `zipfile`.

The COMBINE archives of `crates/core/tests/data/omex/` do not cover every corner of the zip
container the Rust reader has to handle, so the corners get their own small fixtures in
`crates/core/tests/data/zip/`, written here rather than assembled byte by byte in Rust: what
the reference implementation writes is better evidence than bytes we made up.

    stored.zip       one stored member
    zip64.zip        the zip64 end-of-central-directory record, locator and extra field
    descriptors.zip  members written through an unseekable stream, so they carry a data
                     descriptor and a local header with zeroed sizes and bit 3 set
    bzip2.zip        a member compressed with bzip2 (method 12), which is unsupported
    bad_crc.zip      `stored.zip` with one byte of the member data flipped
    truncated.zip    `stored.zip` without its end-of-central-directory record
    oversized.zip    a central directory which declares sizes the archive cannot hold, and
                     one which declares a member larger than a reader is willing to inflate
    understated.zip  a copy of `CompModels.omex` whose central directory declares fewer
                     uncompressed bytes for one member than its deflate stream holds
    prepended.zip    a COMBINE archive behind a self extracting stub, so that every offset of
                     its central directory is off by the length of the stub
    prepended_zip64.zip
                     `zip64.zip` behind the same stub, where the offset of the zip64 record is
                     off by it too
    bad_offset.zip   a central directory which claims to start behind where it ends

The members are stored and not deflated, and their modification time is fixed, so that the
committed fixtures and a fresh run of this script agree byte for byte; `bzip2.zip` is the one
fixture whose bytes come out of a compressor. Only the standard library is needed. Run it from
a pymetadata checkout so that its environment is used, like the other scripts:

    cd /path/to/pymetadata && uv run python /path/to/pymetadata-core/scripts/zip_edge_fixtures.py
"""

import struct
import zipfile
from pathlib import Path
from typing import BinaryIO

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "crates" / "core" / "tests" / "data" / "zip"
OMEX = ROOT / "crates" / "core" / "tests" / "data" / "omex"

#: the stub `prepended.zip` carries in front of the archive, which holds no zip signature
STUB = (b"#!/bin/sh\n# a self extracting stub, and not a zip archive\n" * 16)[:512]

#: the modification time every member is given, so that the fixtures are reproducible
DATE_TIME = (2026, 1, 1, 0, 0, 0)
#: the value a 32 bit field of the central directory carries when zip64 holds the real one
SENTINEL32 = 0xFFFFFFFF
#: the value the member count of the end-of-central-directory record carries under zip64
SENTINEL16 = 0xFFFF
#: the version a zip64 record needs to be read
ZIP64_VERSION = 45

CENTRAL_SIGNATURE = b"PK\x01\x02"
EOCD_SIGNATURE = b"PK\x05\x06"
ZIP64_EOCD_SIGNATURE = b"PK\x06\x06"
ZIP64_LOCATOR_SIGNATURE = b"PK\x06\x07"
#: the fixed part of a central directory file header
CENTRAL_SIZE = 46
#: the fixed part of an end-of-central-directory record
EOCD_SIZE = 22


class WriteOnly:
    """A write only stream, which makes `zipfile` fall back to data descriptors.

    `ZipFile` probes the stream with `tell()`; a stream without one is taken for a pipe, and
    every member it writes then carries bit 3 of the general purpose flags, a local header with
    zeroed CRC and sizes, and a data descriptor behind its data.
    """

    def __init__(self, path: Path) -> None:
        """Open `path` for writing.

        Args:
            path: The file to write.
        """
        self._handle = path.open("wb")

    def write(self, data: bytes) -> int:
        """Write `data`.

        Args:
            data: The bytes to write.

        Returns:
            The number of bytes written.
        """
        return self._handle.write(data)

    def flush(self) -> None:
        """Flush the stream."""
        self._handle.flush()

    def close(self) -> None:
        """Close the stream."""
        self._handle.close()


def info(name: str, method: int = zipfile.ZIP_STORED) -> zipfile.ZipInfo:
    """Describe one member with a fixed modification time.

    Args:
        name: The member name.
        method: The compression method.

    Returns:
        The member description.
    """
    entry = zipfile.ZipInfo(name, date_time=DATE_TIME)
    entry.compress_type = method
    entry.create_system = 3
    return entry


def pattern(length: int) -> bytes:
    """Build `length` deterministic bytes.

    Args:
        length: The number of bytes.

    Returns:
        The bytes, a repeating byte pattern.
    """
    return bytes(index % 251 for index in range(length))


def write_archive(path: Path, members: dict[str, bytes], method: int = zipfile.ZIP_STORED) -> None:
    """Write an archive with one entry per member.

    Args:
        path: The archive to write.
        members: The member names and their content.
        method: The compression method of every member.
    """
    with zipfile.ZipFile(path, "w") as archive:
        for name, content in members.items():
            archive.writestr(info(name, method), content)


def write_zip64(path: Path, members: dict[str, bytes]) -> None:
    """Write an archive which needs zip64 to be read.

    The members are written with `force_zip64`, so every local header holds 0xffffffff sizes and
    a zip64 extra field with the real ones. The central directory and the
    end-of-central-directory record of such a small archive hold the real 32 bit values, so they
    are rewritten here: every record carries 0xffffffff sizes and local header offset plus a
    zip64 extra field, and the record at the end is a zip64 end-of-central-directory record, its
    locator, and an end-of-central-directory record of sentinels. That is the layout of an
    archive above 4 GB, which `zipfile` cannot be asked for without writing 4 GB.

    Args:
        path: The archive to write.
        members: The member names and their content.
    """
    with zipfile.ZipFile(path, "w") as archive:
        for name, content in members.items():
            with archive.open(info(name), "w", force_zip64=True) as handle:
                handle.write(content)
    path.write_bytes(to_zip64(path.read_bytes()))


def to_zip64(raw: bytes) -> bytes:
    """Rewrite the central directory and the record at the end of an archive for zip64.

    Args:
        raw: The archive as `zipfile` wrote it, with one disk and no archive comment.

    Returns:
        The same members under a zip64 central directory, a zip64 end-of-central-directory
        record and its locator.

    Raises:
        ValueError: If the archive is not the single disk, comment free one this expects.
    """
    eocd = raw.rfind(EOCD_SIGNATURE)
    if eocd < 0 or eocd + EOCD_SIZE != len(raw):
        raise ValueError("the archive has no end-of-central-directory record at its end")
    count, _size, offset = struct.unpack("<HII", raw[eocd + 10 : eocd + 20])
    central = bytearray()
    cursor = offset
    for _ in range(count):
        if raw[cursor : cursor + 4] != CENTRAL_SIGNATURE:
            raise ValueError(f"no central directory file header at {cursor}")
        fixed = bytearray(raw[cursor : cursor + CENTRAL_SIZE])
        names, extras, comments = struct.unpack("<HHH", fixed[28:34])
        tail = cursor + CENTRAL_SIZE
        name = raw[tail : tail + names]
        extra = raw[tail + names : tail + names + extras]
        comment = raw[tail + names + extras : tail + names + extras + comments]
        compressed, uncompressed = struct.unpack("<II", fixed[20:28])
        header_offset = struct.unpack("<I", fixed[42:46])[0]
        extra = struct.pack("<HHQQQ", 1, 24, uncompressed, compressed, header_offset) + extra
        struct.pack_into("<H", fixed, 6, ZIP64_VERSION)
        struct.pack_into("<II", fixed, 20, SENTINEL32, SENTINEL32)
        struct.pack_into("<H", fixed, 30, len(extra))
        struct.pack_into("<I", fixed, 42, SENTINEL32)
        central += fixed + name + extra + comment
        cursor = tail + names + extras + comments
    if cursor != eocd:
        raise ValueError("the central directory does not end at the end record")
    zip64_eocd = offset + len(central)
    record = struct.pack(
        "<4sQHHIIQQQQ",
        ZIP64_EOCD_SIGNATURE,
        44,  # the size of this record behind this field
        ZIP64_VERSION,
        ZIP64_VERSION,
        0,
        0,
        count,
        count,
        len(central),
        offset,
    )
    locator = struct.pack("<4sIQI", ZIP64_LOCATOR_SIGNATURE, 0, zip64_eocd, 1)
    end = struct.pack(
        "<4sHHHHIIH",
        EOCD_SIGNATURE,
        0,
        0,
        SENTINEL16,
        SENTINEL16,
        SENTINEL32,
        SENTINEL32,
        0,
    )
    return raw[:offset] + bytes(central) + record + locator + end


def write_descriptors(path: Path, members: dict[str, bytes]) -> None:
    """Write an archive through an unseekable stream, so its members carry data descriptors.

    Args:
        path: The archive to write.
        members: The member names and their content.
    """
    stream = WriteOnly(path)
    try:
        with zipfile.ZipFile(stream, "w") as archive:
            for name, content in members.items():
                archive.writestr(info(name), content)
    finally:
        stream.close()
    with open(path, "rb") as handle:
        flags = local_flags(handle)
    if not flags or any(flag & 0x0008 == 0 for flag in flags):
        raise ValueError(f"zipfile wrote no data descriptor, local flags {flags}")


def local_flags(handle: BinaryIO) -> list[int]:
    """Read the general purpose flags of every local file header of an archive.

    Args:
        handle: The open archive.

    Returns:
        The flags, in the order of the central directory.
    """
    with zipfile.ZipFile(handle) as archive:
        flags = []
        for entry in archive.infolist():
            handle.seek(entry.header_offset + 6)
            flags.append(struct.unpack("<H", handle.read(2))[0])
        return flags


def data_offset(raw: bytes, header_offset: int = 0) -> int:
    """Find where the data of a member starts.

    Args:
        raw: The archive.
        header_offset: The offset of the local file header of the member.

    Returns:
        The offset of the first byte of the member data.
    """
    names, extras = struct.unpack("<HH", raw[header_offset + 26 : header_offset + 30])
    return header_offset + 30 + names + extras


def patch_central(raw: bytes, index: int, offset: int, packed: bytes) -> bytes:
    """Overwrite bytes of one central directory file header.

    Args:
        raw: The archive, with the central directory at its end before the end record.
        index: The position of the member in the central directory.
        offset: The offset of the field inside the file header.
        packed: The bytes to write there.

    Returns:
        The patched archive.

    Raises:
        ValueError: If the archive holds no such central directory file header.
    """
    eocd = raw.rfind(EOCD_SIGNATURE)
    cursor = struct.unpack("<I", raw[eocd + 16 : eocd + 20])[0]
    for _ in range(index):
        names, extras, comments = struct.unpack("<HHH", raw[cursor + 28 : cursor + 34])
        cursor += CENTRAL_SIZE + names + extras + comments
    if raw[cursor : cursor + 4] != CENTRAL_SIGNATURE:
        raise ValueError(f"no central directory file header {index}")
    patched = bytearray(raw)
    patched[cursor + offset : cursor + offset + len(packed)] = packed
    return bytes(patched)


def central_index(raw: bytes, name: bytes) -> int:
    """Find the position of a member in the central directory of an archive.

    Args:
        raw: The archive.
        name: The name of the member.

    Returns:
        The position of the member in the central directory.

    Raises:
        ValueError: If the archive lists no member of that name.
    """
    eocd = raw.rfind(EOCD_SIGNATURE)
    count = struct.unpack("<H", raw[eocd + 10 : eocd + 12])[0]
    cursor = struct.unpack("<I", raw[eocd + 16 : eocd + 20])[0]
    for index in range(count):
        names, extras, comments = struct.unpack("<HHH", raw[cursor + 28 : cursor + 34])
        if raw[cursor + CENTRAL_SIZE : cursor + CENTRAL_SIZE + names] == name:
            return index
        cursor += CENTRAL_SIZE + names + extras + comments
    raise ValueError(f"no member {name!r} in the central directory")


def patch_eocd(raw: bytes, offset: int, packed: bytes) -> bytes:
    """Overwrite bytes of the end-of-central-directory record of an archive.

    Args:
        raw: The archive.
        offset: The offset of the field inside the record.
        packed: The bytes to write there.

    Returns:
        The patched archive.
    """
    eocd = raw.rfind(EOCD_SIGNATURE)
    patched = bytearray(raw)
    patched[eocd + offset : eocd + offset + len(packed)] = packed
    return bytes(patched)


def main() -> None:
    """Write every fixture."""
    DATA.mkdir(parents=True, exist_ok=True)

    stored = DATA / "stored.zip"
    write_archive(stored, {"stored.txt": b"a stored member of a hand built zip\n"})

    write_zip64(DATA / "zip64.zip", {"first.bin": pattern(10_000), "second.bin": pattern(20_000)})

    write_descriptors(
        DATA / "descriptors.zip",
        {"streamed.txt": b"written through a pipe\n", "second.bin": pattern(5_000)},
    )

    write_archive(DATA / "bzip2.zip", {"bzip2.txt": pattern(4_000)}, zipfile.ZIP_BZIP2)

    raw = stored.read_bytes()
    flipped = bytearray(raw)
    flipped[data_offset(raw)] ^= 0xFF
    (DATA / "bad_crc.zip").write_bytes(bytes(flipped))

    (DATA / "truncated.zip").write_bytes(raw[: -EOCD_SIZE - 10])

    # three members which declare a size the archive cannot deliver: more compressed bytes
    # than the archive holds, more uncompressed bytes than 1032:1 can expand two bytes into,
    # and more uncompressed bytes than a reader will commit memory for. None of the three may
    # be allocated for, and none may be read past the end of the archive.
    oversized = DATA / "oversized.zip"
    write_archive(
        oversized,
        {
            "huge_compressed.txt": b"short\n",
            "huge_uncompressed.txt": b"?\n",
            "beyond_the_cap.txt": b"?\n",
        },
    )
    patched = patch_central(oversized.read_bytes(), 0, 20, struct.pack("<I", 0xFFFFFFF0))
    patched = patch_central(patched, 1, 10, struct.pack("<H", 8))
    patched = patch_central(patched, 1, 24, struct.pack("<I", 100_000_000))
    patched = patch_central(patched, 2, 10, struct.pack("<H", 8))
    patched = patch_central(patched, 2, 24, struct.pack("<I", 0x7FFFFFF0))
    oversized.write_bytes(patched)

    # a COMBINE archive which was appended to something else: `CompModels.omex` is the one of
    # the corpus whose members carry data descriptors, so every kind of offset moves at once
    source = OMEX / "CompModels.omex"
    (DATA / "prepended.zip").write_bytes(STUB + source.read_bytes())
    zip64 = DATA / "zip64.zip"
    (DATA / "prepended_zip64.zip").write_bytes(STUB + zip64.read_bytes())

    # a deflated member whose central directory understates how much its stream inflates to.
    # The stream is the one of the committed archive, so no compressor runs here.
    index = central_index(source.read_bytes(), b"README.md")
    understated = patch_central(source.read_bytes(), index, 24, struct.pack("<I", 10))
    (DATA / "understated.zip").write_bytes(understated)

    # the end-of-central-directory record of `stored.zip` with the offset of the central
    # directory moved 100 bytes behind the record itself, which no stub can explain
    offset = struct.unpack("<I", raw[raw.rfind(EOCD_SIGNATURE) + 16 :][:4])[0]
    (DATA / "bad_offset.zip").write_bytes(patch_eocd(raw, 16, struct.pack("<I", offset + 100)))

    verify(source)


def members_of(path: Path) -> tuple[list[str], dict[str, bytes]]:
    """Read an archive with `zipfile`.

    Args:
        path: The archive to read.

    Returns:
        The member names in central directory order, and the content per name.

    Raises:
        ValueError: If `zipfile` reports a bad member.
    """
    with zipfile.ZipFile(path) as archive:
        if archive.testzip() is not None:
            raise ValueError(f"{path.name}: zipfile reports a bad member")
        names = [entry.filename for entry in archive.infolist()]
        return names, {entry.filename: archive.read(entry) for entry in archive.infolist()}


def verify_prepended(fixture: Path, source: Path) -> None:
    """Check that the stub changes nothing about what `zipfile` reads.

    That is the behaviour the Rust reader has to match: the same member names, in the same
    order, with the same content, however long the data in front of the archive is.

    Args:
        fixture: The archive behind the stub.
        source: The archive it was built from.

    Raises:
        ValueError: If `zipfile` reads the two differently.
    """
    if members_of(fixture) != members_of(source):
        raise ValueError(f"{fixture.name}: zipfile reads it and {source.name} differently")


def verify(prepended_source: Path) -> None:
    """Check the fixtures `zipfile` is meant to read back.

    Args:
        prepended_source: The archive `prepended.zip` was built from.

    Raises:
        ValueError: If `zipfile` does not read a fixture back, or reads other content.
    """
    expected = {
        "stored.zip": {"stored.txt": b"a stored member of a hand built zip\n"},
        "zip64.zip": {"first.bin": pattern(10_000), "second.bin": pattern(20_000)},
        "descriptors.zip": {
            "streamed.txt": b"written through a pipe\n",
            "second.bin": pattern(5_000),
        },
        "bzip2.zip": {"bzip2.txt": pattern(4_000)},
    }
    for name, members in expected.items():
        path = DATA / name
        with zipfile.ZipFile(path) as archive:
            if archive.testzip() is not None:
                raise ValueError(f"{name}: zipfile reports a bad member")
            read = {entry.filename: archive.read(entry) for entry in archive.infolist()}
        if read != members:
            raise ValueError(f"{name}: zipfile reads other content than was written")
    with zipfile.ZipFile(DATA / "zip64.zip") as archive:
        sizes = [(entry.compress_size, entry.file_size) for entry in archive.infolist()]
    if sizes != [(10_000, 10_000), (20_000, 20_000)]:
        raise ValueError(f"zip64.zip: zipfile reads the sizes as {sizes}")
    verify_prepended(DATA / "prepended.zip", prepended_source)
    verify_prepended(DATA / "prepended_zip64.zip", DATA / "zip64.zip")


if __name__ == "__main__":
    main()
