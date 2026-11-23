"""Timings of pymetadata 0.8.3, the python side of `BENCHMARKS.md`.

A benchmark is not a gate: nothing here is asserted and no test runs it.

    uv run --no-sync python scripts/python_timing.py [--csv out.csv] [directory]

The protocol is the one of `crates/core/examples/omex_timing.rs`: 3 warm-up calls, then 3 rounds
of 20 timed calls, and the number is the median of the three round medians. `gc.collect()` runs
before each timed call and is not timed. Every cell returns a checksum which is printed next to
the number, and the sum of the checksums is kept, so a reader can compare runs.

Two kinds of cells. The ones named after the public API of pymetadata (`from_omex`, `to_omex`,
`load_modify_save`) pay what a caller of the library pays: files in a temporary directory, a
pydantic manifest, a write to a path. The others (`manifest_only`, `read_all`,
`zip_write_recompress`) start from the bytes of the archive in memory, as the Rust cells do, and
use `zipfile` and, for the manifest, the pydantic `Manifest` of pymetadata.
"""

from __future__ import annotations

import argparse
import gc
import io
import logging
import shutil
import statistics
import tempfile
import time
import xml.etree.ElementTree as ET
import zipfile
from collections.abc import Callable
from pathlib import Path

from pymetadata.omex import Manifest, ManifestEntry, Omex

WARMUP = 3
ROUNDS = 3
CALLS = 20
# the level pymetadata.Omex.to_omex deflates at, and the one of the Rust write_recompress cell;
# 6 is the default of zlib and of zipfile, and what the spike measured
LEVEL = 9
PAYLOAD = b"x" * (64 * 1024)
MANIFEST = "manifest.xml"
CORPUS = Path(__file__).resolve().parent.parent / "crates/core/tests/data/omex"

# every checksum goes in here, so that no timed call has a result which nothing reads
SINK: list[int] = []


def timeit(fn: Callable[[], int]) -> tuple[float, list[float], int]:
    """Time `fn` by the protocol.

    Args:
        fn: the cell, which returns its checksum.

    Returns:
        The median of the round medians in ms, the round medians, and the last checksum.
    """
    checksum = 0
    for _ in range(WARMUP):
        checksum = fn()
    rounds = []
    for _ in range(ROUNDS):
        calls = []
        for _ in range(CALLS):
            gc.collect()
            start = time.perf_counter()
            checksum = fn()
            calls.append((time.perf_counter() - start) * 1000.0)
            SINK.append(checksum)
        rounds.append(statistics.median(calls))
    return statistics.median(rounds), rounds, checksum


def manifest_only(data: bytes) -> int:
    """Read `manifest.xml` of the archive and parse it into the pydantic `Manifest`.

    Args:
        data: the bytes of the archive.

    Returns:
        The number of entries of the manifest.
    """
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        xml = zf.read(MANIFEST)
    root = ET.fromstring(xml)
    entries = [dict(content.attrib) for content in root.findall("{*}content")]
    return len(Manifest(entries=entries))


def read_all(data: bytes) -> int:
    """Inflate every file of the archive but the manifest.

    Args:
        data: the bytes of the archive.

    Returns:
        The number of bytes inflated.
    """
    total = 0
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        for info in zf.infolist():
            if info.is_dir() or info.filename == MANIFEST:
                continue
            total += len(zf.read(info))
    return total


def zip_write_recompress(data: bytes, level: int) -> int:
    """Open the archive and write every file again at `level`, in memory.

    This is the work of `to_omex` without the temporary directory and the path.

    Args:
        data: the bytes of the archive.
        level: the deflate level.

    Returns:
        The size of the written archive.
    """
    out = io.BytesIO()
    with (
        zipfile.ZipFile(io.BytesIO(data)) as zf,
        zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=level) as zo,
    ):
        for info in zf.infolist():
            if not info.is_dir():
                zo.writestr(info.filename, zf.read(info))
    return len(out.getvalue())


def api_cells(
    path: Path, omex: Omex, target: ManifestEntry, payload_path: Path, out: Path
) -> list[tuple[str, Callable[[], int]]]:
    """The cells which go through the public API of pymetadata.

    Args:
        path: the archive.
        omex: the archive, loaded once, which `to_omex` writes.
        target: the entry `load_modify_save` replaces.
        payload_path: the file with the new content of the entry.
        out: where the archives are written, and removed again.

    Returns:
        The names of the cells and the cells.
    """

    def from_omex() -> int:
        loaded = Omex.from_omex(path)
        try:
            return len(loaded.manifest)
        finally:
            loaded.close()

    def to_omex() -> int:
        omex.to_omex(out)
        size = out.stat().st_size
        out.unlink()
        return size

    def load_modify_save() -> int:
        loaded = Omex.from_omex(path)
        try:
            loaded.add_entry(payload_path, target.model_copy())
            loaded.to_omex(out)
            size = out.stat().st_size
            out.unlink()
            return size
        finally:
            loaded.close()

    return [
        ("from_omex", from_omex),
        ("to_omex", to_omex),
        ("load_modify_save", load_modify_save),
    ]


def main() -> None:
    """Run every cell over every archive and print a markdown table."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("directory", nargs="?", type=Path, default=CORPUS)
    parser.add_argument("--csv", type=Path, help="also write the raw numbers")
    args = parser.parse_args()
    # add_entry warns about the entry it replaces, once per call
    logging.getLogger("pymetadata").setLevel(logging.ERROR)

    rows: list[tuple[str, int, int, str, float, list[float], int]] = []
    work = Path(tempfile.mkdtemp(prefix="python_timing_"))
    try:
        payload_path = work / "payload.bin"
        payload_path.write_bytes(PAYLOAD)
        for path in sorted(args.directory.glob("*.omex")):
            data = path.read_bytes()
            entries = manifest_only(data)
            omex = Omex.from_omex(path)
            with zipfile.ZipFile(path) as zf:
                held = {i.filename for i in zf.infolist() if not i.is_dir()}
            # replace the first entry which has a file, as the Rust cell does
            target = next(
                e.model_copy()
                for e in omex.manifest.entries
                if e.location not in {".", "./manifest.xml"}
                and e.location.removeprefix("./") in held
            )
            cells: list[tuple[str, Callable[[], int]]] = [
                ("manifest_only", lambda d=data: manifest_only(d)),
                ("read_all", lambda d=data: read_all(d)),
                *api_cells(path, omex, target, payload_path, work / "out.omex"),
                ("zip_write_recompress", lambda d=data: zip_write_recompress(d, 9)),
                ("zip_write_recompress_l6", lambda d=data: zip_write_recompress(d, 6)),
            ]
            for op, fn in cells:
                ms, round_medians, checksum = timeit(fn)
                rows.append(
                    (path.name, len(data), entries, op, ms, round_medians, checksum)
                )
            omex.close()
    finally:
        shutil.rmtree(work, ignore_errors=True)

    print("| archive | size | entries | op | python ms | checksum |")
    print("|---|---|---|---|---|---|")
    for name, size, entries, op, ms, _, checksum in rows:
        shown = (
            f"{size / 1e6:.1f} MB" if size >= 1_000_000 else f"{-(-size // 1000)} kB"
        )
        print(f"| {name} | {shown} | {entries} | {op} | {ms:.4f} | {checksum} |")

    if args.csv:
        lines = [
            "archive,size_bytes,entries,op,ms,round1_ms,round2_ms,round3_ms,checksum"
        ]
        for name, size, entries, op, ms, r, checksum in rows:
            lines.append(
                f"{name},{size},{entries},{op},{ms:.4f},{r[0]:.4f},{r[1]:.4f},{r[2]:.4f},{checksum}"
            )
        args.csv.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
