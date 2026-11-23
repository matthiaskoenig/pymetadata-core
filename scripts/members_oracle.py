"""Write the zip member fixtures of the core from python's `zipfile`.

For each COMBINE archive of `crates/core/tests/data/omex/` this reads the members with
python's `zipfile` and writes them to `crates/core/tests/data/members.json`: per archive the
list of `{name, method, crc32, size, sha256}` of its members, in central directory order, where
`sha256` is the digest of the uncompressed bytes. The Rust zip reader is checked against that
list, so what it inflates is what python reads. The archives are the committed copies which
`scripts/manifest_parity.py` took from the pymetadata test data, so that the digests and the
fixtures they describe cannot drift apart.

Only the standard library is needed. Run it from a pymetadata checkout so that its environment
is used, like the other scripts:

    cd /path/to/pymetadata && uv run python /path/to/pymetadata-core/scripts/members_oracle.py
"""

import hashlib
import json
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "crates" / "core" / "tests" / "data"


def members(path: Path) -> list[dict[str, object]]:
    """Describe the members of one archive.

    Args:
        path: The archive to read.

    Returns:
        One dict per member, in central directory order, with its name, compression method,
        CRC32, uncompressed size and the SHA256 of its uncompressed bytes.
    """
    with zipfile.ZipFile(path) as archive:
        return [
            {
                "name": info.filename,
                "method": info.compress_type,
                "crc32": info.CRC,
                "size": info.file_size,
                "sha256": hashlib.sha256(archive.read(info)).hexdigest(),
            }
            for info in archive.infolist()
        ]


def main() -> None:
    """Write `members.json` for every archive of the corpus."""
    expected = {
        path.name: members(path) for path in sorted((DATA / "omex").glob("*.omex"))
    }
    if not expected:
        raise SystemExit(f"no archive in {DATA / 'omex'}")
    (DATA / "members.json").write_text(
        json.dumps(expected, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
