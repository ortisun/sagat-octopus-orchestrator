# runners/lake

## Overview

The Python runtime of the Iceberg write steps and destinations (feature 035, D035-026). The Rust lake runner owns the
lease, the credentials and the only network route; this program owns Iceberg semantics through PyIceberg, inside a
per-attempt sandbox container (`specs/035-iceberg-write-steps/contracts/lake-runtime.md`).

This directory holds the image, its lock and its checks (T035-006), the filter of what a catalog sends
(`config_hygiene.py`), the loopback ports that reach the relay (`forwarder.py`) and the host's side of `sagat.lake.v1`
(`host.py`). The host serves no operation yet: the write protocol and the receipts arrive with the tasks that test them.
Three decisions of the write are already pure functions with their own tests: `mapping.py` (a record against the schema of
the table, lossless or refused; the table is published in `docs/guides/iceberg-type-mapping.md`), `lookup.py` (the search for
the effect key, D035-030) and `compat.py` (whether the table can be written, FR-035-011). `table_facts.py` reads their
inputs from a table PyIceberg loaded and from nothing else (D035-092). `commit.py` makes PyIceberg send the commit once
instead of repeating a refused one on its own (D035-093); `tests/counting_catalog.py` is the REST catalog that counts the
commit requests it receives.

## Interface

- `Dockerfile`, stage `lake-runtime`: Python 3.12 slim pinned by digest; the closure of `requirements.lock` installed
  from wheels with `--require-hashes --no-deps --only-binary=:all:`; pip and setuptools removed; no compiler; the host
  at `/opt/sagat/lake`, read-only; user `65534`.
- `requirements.in` is the one decided requirement (`pyiceberg[pyarrow,s3fs,gcsfs,adlfs,hive,glue,sql-postgres]==0.12.0`);
  `requirements.lock` is its closure with hashes (78 packages). `requirements-dev.in` and `requirements-dev.lock` add
  pytest, mypy, ruff and pip-audit for CI and for you.
- `sagat_lake/`: the host package. `host.py` is the entry point (`python3 -I -X utf8 /opt/sagat/lake/sagat_lake/host.py
  --attempt-dir <dir>`); `forwarder.py` turns an HTTP proxy port and a TCP port per mapping into connections to the relay;
  `config_hygiene.py` filters what a catalog sends; `failures.py` is the closed failure a result line may carry.
- `smoke-image.sh <image>`: starts the image the way the sandbox runner does (no network, read-only root, no
  capabilities) and checks what the contract promises of it.
- `tools/check_sbom.py <image.tar> <package>...`: reads the SPDX attestation BuildKit puts in an image exported as an
  OCI archive and fails when it is missing or lacks a named package. A CI tool; it is not copied into the image.

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

CI (`.github/workflows/ci.yml`, job `lake-runtime`): the unit tests and the audit of the lock run when a file other than
Markdown under `runners/lake/` changes; the types and lint, the image, `smoke-image.sh` and the SBOM check run for a
release (Constitution 4.1.0). `.github/workflows/lake-audit.yml` audits the lock every week. A change under
`runners/lake/` selects the lake suites (`iceberg-writes`), not every integration target.

Regenerate a lock after changing its `.in` file, from `runners/lake`:

```sh
uv pip compile requirements.in --generate-hashes --universal --python-version 3.12 --exclude-newer <date> --no-annotate -o requirements.lock
uv pip compile requirements-dev.in --generate-hashes --universal --python-version 3.12 --exclude-newer <date> --no-annotate -o requirements-dev.lock
```

Delete `.venv` and the image archive when you finish; they are large.
