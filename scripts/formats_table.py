"""Generate the format table of the core from `pymetadata.omex`.

Writes `crates/core/src/formats_table.rs` (data only, the logic is the hand
written `formats.rs`, formatted with `rustfmt`) and `crates/core/tests/data/formats.json`,
so the python and the Rust format tables never drift. Run it from a pymetadata
checkout so that its environment is used:

    cd /path/to/pymetadata && uv run python /path/to/pymetadata-core/scripts/formats_table.py
"""

import json
import subprocess
from pathlib import Path

from pymetadata.omex import _XML_ROOT_FORMATS, EntryFormat

ROOT = Path(__file__).resolve().parent.parent
FORMATS_RS = ROOT / "crates" / "core" / "src" / "formats_table.rs"
FORMATS_JSON = ROOT / "crates" / "core" / "tests" / "data" / "formats.json"

HEADER = """\
// This file is generated, do not edit. Source: pymetadata.omex (EntryFormat and
// _XML_ROOT_FORMATS), generator: scripts/formats_table.py.

//! The format tables of COMBINE archives, read by `formats`.
"""


def rust_str(text: str) -> str:
    """Return `text` as a Rust string literal.

    Args:
        text: the text

    Returns:
        The quoted literal.
    """
    escaped = text.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def main() -> None:
    """Write the Rust table and the JSON fixture."""
    # `__members__` includes the aliases (`RDF` for `XML`), like `hasattr` in `lookup_format`
    pairs = {
        name.lower(): str(member.value)
        for name, member in EntryFormat.__members__.items()
    }

    lines = [HEADER]
    lines.append(
        "/// Every format key (the lower case name of an `EntryFormat` member) and its URI."
    )
    lines.append("pub(crate) const FORMATS: &[(&str, &str)] = &[")
    for key, uri in pairs.items():
        lines.append(f"    ({rust_str(key)}, {rust_str(uri)}),")
    lines.append("];\n")
    lines.append(
        "/// Start tags in the head of `.xml` files and the format keys they stand for, in order."
    )
    lines.append("pub(crate) const XML_ROOT_FORMATS: &[(&[u8], &str)] = &[")
    for tag, key in _XML_ROOT_FORMATS:
        lines.append(f"    ({rust_str(tag.decode())}.as_bytes(), {rust_str(key)}),")
    lines.append("];")
    FORMATS_RS.write_text("\n".join(lines) + "\n", encoding="utf-8")
    # fails loudly if rustfmt is missing or rejects the file
    subprocess.run(["rustfmt", "--edition", "2024", str(FORMATS_RS)], check=True)

    FORMATS_JSON.write_text(json.dumps(pairs, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
