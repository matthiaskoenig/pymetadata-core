"""Fixtures of the tests of `pymetadata_core`."""

from pathlib import Path

import pytest

DATA = Path(__file__).parents[2] / "crates" / "core" / "tests" / "data"
"""The fixtures of the core, shared with its rust tests."""

ARCHIVES = sorted((DATA / "omex").glob("*.omex"))
"""The four COMBINE archives of the corpus."""


@pytest.fixture(params=ARCHIVES, ids=lambda path: path.stem)
def archive_path(request: pytest.FixtureRequest) -> Path:
    """Each archive of the corpus in turn."""
    return request.param
