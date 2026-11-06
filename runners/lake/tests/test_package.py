"""The host package and the environment it runs in (the closure of the lock, installed)."""

from __future__ import annotations

import importlib
import importlib.metadata
import tomllib
from pathlib import Path

import pytest

import sagat_lake


def test_the_package_names_the_version_of_pyproject(lake_root: Path) -> None:
    project = tomllib.loads((lake_root / "pyproject.toml").read_text(encoding="utf-8"))["project"]
    assert sagat_lake.__version__ == project["version"]


def test_the_environment_has_the_pinned_pyiceberg() -> None:
    assert importlib.metadata.version("pyiceberg") == "0.12.0"


@pytest.mark.parametrize(
    "module",
    [
        "pyiceberg.io.pyarrow",  # extra pyarrow: the file IO that reads and writes the data files
        "s3fs",  # extra s3fs: S3-compatible stores
        "gcsfs",  # extra gcsfs: Google Cloud Storage
        "adlfs",  # extra adlfs: Azure Data Lake Storage
        "pyiceberg.catalog.rest",  # the REST catalog, the reference one
        "pyiceberg.catalog.hive",  # extra hive: thrift
        "pyiceberg.catalog.glue",  # extra glue: boto3
        "pyiceberg.catalog.sql",  # extra sql-postgres: SQLAlchemy and psycopg2
        "psycopg2",
    ],
)
def test_every_storage_and_catalog_extra_imports(module: str) -> None:
    importlib.import_module(module)
