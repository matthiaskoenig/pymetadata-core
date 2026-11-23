"""Write the manifest fixtures of the core from `pymetadata.omex`.

For each archive of `tests/data/omex/` of pymetadata this copies the archive to
`crates/core/tests/data/omex/`, writes its `manifest.xml` to
`crates/core/tests/data/manifests/<archive>.xml` and the entries (location, format, master)
which `Manifest.from_manifest` reads from it to `crates/core/tests/data/manifests.json`, so the
Rust manifest can be checked against the python one. Run it from a pymetadata checkout so
that its environment is used:

    cd /path/to/pymetadata && uv run python /path/to/pymetadata-core/scripts/manifest_parity.py
"""

import json
import shutil
import zipfile
from pathlib import Path

from pymetadata.omex import Manifest

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "crates" / "core" / "tests" / "data"
SOURCE = Path.cwd() / "tests" / "data" / "omex"


def main() -> None:
    """Copy the archives and write the manifest fixtures."""
    (DATA / "omex").mkdir(parents=True, exist_ok=True)
    (DATA / "manifests").mkdir(parents=True, exist_ok=True)
    expected: dict[str, list[dict[str, object]]] = {}
    for path in sorted(SOURCE.glob("*.omex")):
        shutil.copy(path, DATA / "omex" / path.name)
        with zipfile.ZipFile(path) as archive:
            manifest_xml = archive.read("manifest.xml")
        (DATA / "manifests" / f"{path.stem}.xml").write_bytes(manifest_xml)
        manifest = Manifest.from_manifest(DATA / "manifests" / f"{path.stem}.xml")
        expected[path.name] = [
            {"location": e.location, "format": e.format, "master": e.master}
            for e in manifest.entries
        ]
    (DATA / "manifests.json").write_text(
        json.dumps(expected, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
