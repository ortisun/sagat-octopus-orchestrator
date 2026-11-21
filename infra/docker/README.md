# Local Compose infrastructure

## Overview

Development containers and PostgreSQL first-initialization hooks. Compose configuration and observability overlays remain the deployment entry points.

## Interface

The child contracts define the source boundaries in this directory.

## Subdirectories & Boundaries

- [postgres-init](./postgres-init/README.md) — Development runtime-role creation.

## Invariants

- No additional executable invariant is introduced by this grouping; source checks and contract requirements are documented in the linked components.

## Side Effects

- None from the directory grouping itself. The linked component contracts describe command execution, storage and network effects.

## Verification

Run from the repository root:

```sh
bash scripts/smoke-compose.sh
```

These are verification entry points; no runtime test result is implied by this index.
