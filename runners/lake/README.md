# runners/lake

## Overview

The Python runtime of the Iceberg write steps and destinations (feature 035, D035-026). The Rust lake runner owns the
lease, the credentials and the only network route; this program owns Iceberg semantics through PyIceberg, inside a
per-attempt sandbox container (`specs/035-iceberg-write-steps/contracts/lake-runtime.md`).

This directory is the skeleton (T035-006): the image, its lock and its checks. The relay client, the write protocol and
the receipts are not written yet and arrive with the tasks that test them.

## Interface

- `Dockerfile`, stage `lake-runtime`: Python 3.12 slim pinned by digest; the closure of `requirements.lock` installed
  from wheels with `--require-hashes --no-deps --only-binary=:all:`; pip and setuptools removed; no compiler; the host
  at `/opt/sagat/lake`, read-only; user `65534`.
- `requirements.in` is the one decided requirement (`pyiceberg[pyarrow,s3fs,gcsfs,adlfs,hive,glue,sql-postgres]==0.12.0`);
  `requirements.lock` is its closure with hashes (78 packages). `requirements-dev.in` and `requirements-dev.lock` add
  pytest, mypy, ruff and pip-audit for CI and for you.
- `sagat_lake/`: the host package (the version only, for now).
- `smoke-image.sh <image>`: starts the image the way the sandbox runner does (no network, read-only root, no
  capabilities) and checks what the contract promises of it.

## Invariants

- The lock holds only pinned requirements with `sha256` hashes, from the one index; a line that is anything else fails
  `tests/test_lock.py`. [runners/lake/tests/lockfile.py:parse_lock]
- The Dockerfile pins every base by digest, installs with hashes and wheels only, removes pip, adds no compiler or
  package fetch, makes the host read-only and does not run as root. [runners/lake/tests/dockerfile.py:violations]
- A package without a wheel for the platform fails the image build instead of pulling in a compiler.

## Verification

From `runners/lake`, in a virtual environment made from the dev lock:

```sh
python3 -m venv .venv && .venv/bin/pip install --require-hashes --no-deps -r requirements-dev.lock
.venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check . && .venv/bin/ruff format --check .
.venv/bin/pip-audit -r requirements.lock --require-hashes --disable-pip
```

From the repository root, with Docker:

```sh
docker build -f runners/lake/Dockerfile --target lake-runtime -t sagat-lake-runtime .
runners/lake/smoke-image.sh sagat-lake-runtime
# The SBOM: BuildKit's SPDX attestation of the build, with the scanner pinned by digest.
docker buildx build -f runners/lake/Dockerfile --target lake-runtime --sbom=generator=docker/buildkit-syft-scanner:1@sha256:873a7458c3c8ac8e32c7b796c10e1600b07b963d2aa000a72a2d97aaaa2d6315 --output type=oci,dest=lake-runtime.oci.tar .
```

Regenerate a lock after changing its `.in` file, from `runners/lake`:

```sh
uv pip compile requirements.in --generate-hashes --universal --python-version 3.12 --exclude-newer <date> --no-annotate -o requirements.lock
uv pip compile requirements-dev.in --generate-hashes --universal --python-version 3.12 --exclude-newer <date> --no-annotate -o requirements-dev.lock
```

Delete `.venv` and the image archive when you finish; they are large.
