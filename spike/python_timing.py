"""Spike timings: python side. Same protocol as the rust spike."""

from __future__ import annotations

import gc
import io
import shutil
import statistics
import sys
import tempfile
import time
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

WARMUP = 3
ROUNDS = 3
CALLS = 20


def timeit(fn):
    cs = 0
    for _ in range(WARMUP):
        cs = fn()
    round_medians = []
    for _ in range(ROUNDS):
        calls = []
        for _ in range(CALLS):
            gc.collect()
            t0 = time.perf_counter()
            cs = fn()
            calls.append((time.perf_counter() - t0) * 1000.0)
        round_medians.append(statistics.median(calls))
    return statistics.median(round_medians), cs


def manifest_only(data: bytes) -> int:
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        xml = zf.read("manifest.xml")
    root = ET.fromstring(xml)
    n = 0
    for content in root:
        if content.tag.endswith("content"):
            content.attrib["location"]
            content.attrib["format"]
            content.attrib.get("master") in ("true", "1")
            n += 1
    return n


def read_all(data: bytes) -> int:
    total = 0
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        for info in zf.infolist():
            if info.is_dir():
                continue
            total += len(zf.read(info))
    return total


def extract_all(data: bytes) -> int:
    tmp = Path(tempfile.mkdtemp(prefix="spike_py_"))
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as zf:
            zf.extractall(tmp)
        return sum(p.stat().st_size for p in tmp.rglob("*") if p.is_file())
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def write_recompress(data: bytes) -> int:
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zo:
            for info in zf.infolist():
                if info.is_dir():
                    continue
                zo.writestr(info.filename, zf.read(info))
    return len(out.getvalue())


def modify_write(data: bytes, target: str, payload: bytes) -> int:
    """python has no raw copy: a modification re-deflates every entry."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zo:
            for info in zf.infolist():
                if info.is_dir() or info.filename == target:
                    continue
                zo.writestr(info.filename, zf.read(info))
            zo.writestr(target, payload)
    return len(out.getvalue())


def main() -> None:
    from pymetadata.omex import Omex

    data_dir = Path(
        sys.argv[1] if len(sys.argv) > 1 else "/home/mkoenig/git/pymetadata/tests/data/omex"
    )
    paths = sorted(p for p in data_dir.iterdir() if p.suffix == ".omex")
    print("archive,size_bytes,entries,op,ms,checksum")
    for path in paths:
        data = path.read_bytes()
        name = path.name
        entries = manifest_only(data)
        payload = b"x" * 64 * 1024
        with zipfile.ZipFile(io.BytesIO(data)) as zf:
            target = next(
                i.filename
                for i in zf.infolist()
                if not i.is_dir() and i.filename != "manifest.xml"
            )

        for op, fn in (
            ("manifest_only", lambda: manifest_only(data)),
            ("read_all", lambda: read_all(data)),
            ("extract_all", lambda: extract_all(data)),
            ("modify_write", lambda: modify_write(data, target, payload)),
            ("write_recompress", lambda: write_recompress(data)),
        ):
            ms, cs = timeit(fn)
            print(f"{name},{len(data)},{entries},{op},{ms:.4f},{cs}")

        # end to end through the pymetadata API
        def e2e_from_omex() -> int:
            omex = Omex.from_omex(path)
            n = len(omex.manifest)
            omex.close()
            return n

        ms, cs = timeit(e2e_from_omex)
        print(f"{name},{len(data)},{entries},pymetadata_from_omex,{ms:.4f},{cs}")

        omex = Omex.from_omex(path)
        out_dir = Path(tempfile.mkdtemp(prefix="spike_e2e_"))

        def e2e_to_omex() -> int:
            target_path = out_dir / "out.omex"
            omex.to_omex(target_path)
            n = target_path.stat().st_size
            target_path.unlink()
            return n

        ms, cs = timeit(e2e_to_omex)
        print(f"{name},{len(data)},{entries},pymetadata_to_omex,{ms:.4f},{cs}")
        omex.close()
        shutil.rmtree(out_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
