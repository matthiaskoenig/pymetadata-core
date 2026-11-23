"""Turn the runs under `bench/runs/` into the tables of `BENCHMARKS.md` and `bench/RESULTS.md`.

    python scripts/bench_table.py comparison   # rust against python
    python scripts/bench_table.py from_omex    # python from_omex, rust input copy
    python scripts/bench_table.py floor        # the manifest floor
    python scripts/bench_table.py check        # every table is in its document
    python scripts/bench_table.py spread       # the range over the runs of the headline cells
    python scripts/bench_table.py js           # node and chrome

Each harness is run several times, because the machine is shared and a run can overlap a burst
of another job. A cell of a table is the fastest of the runs, each of which is already a
median of round medians; `spread` shows what the other runs said. Nothing here times anything.
"""

from __future__ import annotations

import contextlib
import csv
import io
import json
import math
import sys
from pathlib import Path

RUNS = Path(__file__).resolve().parent.parent / "bench/runs"
ORDER = [
    "CompModels.omex",
    "BIOMD0000000001.omex",
    "CombineArchiveShowCase.omex",
    "iCGB21FR.omex",
]

# label, python cell, rust cell
PAIRS = [
    (
        "manifest_only (rust: from_bytes, input copied)",
        "manifest_only",
        "manifest_only",
    ),
    ("read_all", "read_all", "read_all"),
    (
        "modify, write (python: from_omex, add_entry, to_omex)",
        "load_modify_save",
        "modify_write",
    ),
    ("write unchanged (python: to_omex)", "to_omex", "write_copy"),
    ("recompress, level 9 (python: to_omex)", "to_omex", "write_recompress"),
    (
        "recompress, level 9 (python: zipfile, memory)",
        "zip_write_recompress",
        "write_recompress",
    ),
    (
        "recompress, level 6 (python: zipfile, memory)",
        "zip_write_recompress_l6",
        "write_recompress_l6",
    ),
]


def size(n: int) -> str:
    """A size for a person."""
    return f"{n / 1e6:.1f} MB" if n >= 1_000_000 else f"{-(-n // 1000)} kB"


def factor(x: float) -> str:
    """A factor with two significant digits at most."""
    return f"{x:.1f}x" if x < 10 else f"{x:.0f}x"


def csv_runs(prefix: str) -> tuple[dict[tuple[str, str], list[float]], dict[str, int]]:
    """The milliseconds of every cell over the runs of one csv harness."""
    cells: dict[tuple[str, str], list[float]] = {}
    sizes: dict[str, int] = {}
    for path in sorted(RUNS.glob(f"{prefix}-*.csv")):
        for row in csv.DictReader(path.read_text().splitlines()):
            cells.setdefault((row["archive"], row["op"]), []).append(float(row["ms"]))
            sizes[row["archive"]] = int(row["size_bytes"])
    return cells, sizes


def comparison() -> None:
    """Print the table of rust against python."""
    py, _ = csv_runs("python")
    rs, sizes = csv_runs("rust")
    print("| archive | size | cell | python ms | rust ms | factor |")
    print("|---|---|---|---|---|---|")
    for name in ORDER:
        for label, p, r in PAIRS:
            pv, rv = min(py[(name, p)]), min(rs[(name, r)])
            print(
                f"| {name} | {size(sizes[name])} | {label} | {pv:.3f} | {rv:.3f} | "
                f"{factor(pv / rv)} |"
            )


def from_omex() -> None:
    """Print the python `from_omex` time next to the copy of the input of Rust."""
    py, _ = csv_runs("python")
    rs, _ = csv_runs("rust")
    print("| archive | python from_omex ms | rust input_copy ms |")
    print("|---|---|---|")
    for name in ORDER:
        print(
            f"| {name} | {min(py[(name, 'from_omex')]):.3f} | {min(rs[(name, 'input_copy')]):.4f} |"
        )


def floor() -> None:
    """Print the floor of the manifest request, which no API reaches."""
    py, _ = csv_runs("python")
    rs, _ = csv_runs("rust")
    print(
        "| archive | python manifest_only ms | rust manifest_owned ms | ratio to the floor |"
    )
    print("|---|---|---|---|")
    for name in ORDER:
        pv, rv = min(py[(name, "manifest_only")]), min(rs[(name, "manifest_owned")])
        print(f"| {name} | {pv:.3f} | {rv:.3f} | {factor(pv / rv)} |")


def spread() -> None:
    """Print the range over the runs of the headline cells."""
    py, _ = csv_runs("python")
    rs, _ = csv_runs("rust")
    name = "iCGB21FR.omex"
    print(f"| cell on {name} | runs | fastest ms | slowest ms |")
    print("|---|---|---|---|")
    for side, cells, ops in (
        (
            "rust",
            rs,
            [
                "manifest_only",
                "manifest_owned",
                "modify_write",
                "write_recompress",
                "write_recompress_l6",
            ],
        ),
        (
            "python",
            py,
            ["manifest_only", "to_omex", "load_modify_save", "zip_write_recompress_l6"],
        ),
    ):
        for op in ops:
            v = cells[(name, op)]
            print(f"| {side} {op} | {len(v)} | {min(v):.3f} | {max(v):.3f} |")


def shown_ms(host: str, runs: list[tuple[float, int]] | None) -> str:
    """The best run of a cell, with the precision the measurement has.

    Node times with a clock of nanoseconds. Chrome clamps `performance.now()` to 0.1 ms, so a
    batch of `b` calls resolves 0.1 / b ms, and digits below that are not printed.
    """
    if not runs:
        return "not measured"
    ms, batch = min(runs)
    if host == "node":
        return f"{ms:.4f}"
    # enough decimals to show one quantum: 0.05 needs 2 (0.95), 0.025 needs 2, 0.1 needs 1
    decimals = max(0, math.ceil(-math.log10(0.1 / batch) - 0.05))
    return f"{ms:.{decimals}f}"


def js() -> None:
    """Print the node and chrome tables, one column per host."""
    hosts: dict[str, dict[tuple[str, str], list[tuple[float, int]]]] = {}
    sizes: dict[str, int] = {}
    for host in ("node", "chrome"):
        cells: dict[tuple[str, str], list[tuple[float, int]]] = {}
        for path in sorted(RUNS.glob(f"{host}-*.json")):
            for row in json.loads(path.read_text()):
                cells.setdefault((row["archive"], row["op"]), []).append(
                    (row["ms"], row["batch"])
                )
                sizes[row["archive"]] = row["size"]
        hosts[host] = cells
    print("| archive | size | op | node ms | chrome ms |")
    print("|---|---|---|---|---|")
    for key in hosts["node"]:
        shown = {host: shown_ms(host, cells.get(key)) for host, cells in hosts.items()}
        print(
            f"| {key[0]} | {size(sizes[key[0]])} | {key[1]} | {shown['node']} | {shown['chrome']} |"
        )


def check() -> int:
    """Check that every table a document publishes is the one the runs give.

    Returns:
        0 when every table appears verbatim in its document, 1 otherwise.
    """
    root = RUNS.parent.parent
    docs = {
        "comparison": (comparison, root / "BENCHMARKS.md"),
        "from_omex": (from_omex, root / "BENCHMARKS.md"),
        "floor": (floor, root / "BENCHMARKS.md"),
        "spread": (spread, root / "BENCHMARKS.md"),
        "js": (js, root / "bench/RESULTS.md"),
    }
    failed = 0
    for name, (table, path) in docs.items():
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            table()
        text = buffer.getvalue().strip()
        found = bool(text) and text in path.read_text()
        print(
            f"{name}: {'ok' if found else 'MISSING'} ({len(text.splitlines())} lines in {path.name})"
        )
        failed += not found
    return int(failed > 0)


COMMANDS = {
    "comparison": comparison,
    "from_omex": from_omex,
    "floor": floor,
    "spread": spread,
    "js": js,
}

if __name__ == "__main__":
    if sys.argv[1] == "check":
        sys.exit(check())
    COMMANDS[sys.argv[1]]()
