"""The host package and the environment it runs in (the closure of the lock, installed)."""

from __future__ import annotations

import datetime
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
        "pyiceberg_core",  # extra pyiceberg-core: the native partition transforms
    ],
)
def test_every_storage_and_catalog_extra_imports(module: str) -> None:
    importlib.import_module(module)


def test_the_native_transforms_compute_the_values_of_the_specification() -> None:
    # a wheel that imports but computes nothing, or a different hash, would write rows into the wrong partitions
    import pyarrow as pa  # type: ignore[import-untyped]
    from pyiceberg_core import transform  # type: ignore[import-untyped]

    assert transform.bucket(pa.array([1, 2, 3]), 16).to_pylist() == [4, 4, 3]
    assert transform.truncate(pa.array(["alphabet", "ab"]), 4).to_pylist() == ["alph", "ab"]
    # 19,000 days after 1970-01-01 is 2022-01-08, worked out here and not by the library
    days = pa.array([19_000], pa.date32())
    assert datetime.date(1970, 1, 1) + datetime.timedelta(days=19_000) == datetime.date(2022, 1, 8)
    assert transform.day(days).to_pylist() == [datetime.date(2022, 1, 8)]
    assert transform.month(days).to_pylist() == [(2022 - 1970) * 12]
    assert transform.year(days).to_pylist() == [2022 - 1970]
