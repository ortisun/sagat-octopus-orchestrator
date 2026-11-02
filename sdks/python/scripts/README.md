# scripts

## Overview

Repository synchronization and committed protobuf/catalog generation. [sync_shared.py:main]

## Interface

- generate_proto invokes grpc-tools for Python/types/gRPC outputs; render_catalog produces embedded catalog definitions; main compares or rewrites derived files, with --check for staleness. [sync_shared.py:generate_proto] [sync_shared.py:render_catalog] [sync_shared.py:main] [sync_shared.py:--check]

## Invariants

- Canonical inputs are the platform worker.proto and SDK error-catalog.json. --check reports stale targets and returns 1 without rewriting those targets. [sync_shared.py:CONTRACT_PROTO] [sync_shared.py:CONTRACT_CATALOG] [sync_shared.py:GENERATED] [sync_shared.py:--check]

## Side Effects

- Generation uses temporary staging files and grpc_tools.protoc. Normal mode rewrites proto/bindings/catalog and touches _proto/__init__.py; check mode still generates comparison output in temporary directories, then only reports stale repository targets. [sync_shared.py:generate_proto] [sync_shared.py:main]

## Verification

Run from the repository root:

```sh
cd sdks/python && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
