from __future__ import annotations

import pathlib

import pytest


def _pdf_dir() -> pathlib.Path:
    local = pathlib.Path(__file__).parent / "pdf"
    if local.exists():
        return local
    # Fall back to the Rust tiny-poppler test fixtures in the same workspace.
    workspace = pathlib.Path(__file__).parents[2] / "tiny-poppler" / "tests" / "pdf"
    if workspace.exists():
        return workspace
    return local


@pytest.fixture
def pdf_dir() -> pathlib.Path:
    return _pdf_dir()


def sample(name: str) -> pathlib.Path:
    path = _pdf_dir() / name
    if not path.exists():
        raise FileNotFoundError(f"Missing test PDF: {path}")
    return path
