# Infrastructure

## Overview

Development deployment, managed-credential setup and reproducible PostgreSQL recovery procedures.

## Interface

The child contracts define the source boundaries in this directory.

## Subdirectories & Boundaries

- [docker](./docker/README.md) — Local Compose deployment and database initialization.
- [dr](./dr/README.md) — Disposable restore/failover drill and journal digests.
- [openbao](./openbao/README.md) — OpenBao fixtures, chart bootstrap and scoped policies.

## Invariants

- No additional executable invariant is introduced by this grouping; source checks and contract requirements are documented in the linked components.

## Side Effects

- None from the directory grouping itself. The linked component contracts describe command execution, storage and network effects.

## Verification

Run from the repository root:

```sh
python3 scripts/check-architecture.py
python3 scripts/check-vendor.py
```

These are verification entry points; no runtime test result is implied by this index.
