# PostgreSQL runtime-role initialization

## Overview

First-volume-initialization hook for the development PostgreSQL service.

## Interface

- `01-runtime-role.sh` requires `SAGAT_APP_PASSWORD` and uses `POSTGRES_USER` / `POSTGRES_DB` for its owner connection. [01-runtime-role.sh:SAGAT_APP_PASSWORD] [01-runtime-role.sh:POSTGRES_USER] [01-runtime-role.sh:POSTGRES_DB]
- Creates login role `sagat_app` with `NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION`. [01-runtime-role.sh:CREATE ROLE sagat_app]

## Invariants

- The password is passed as a psql variable and quoted with `:'app_password'`; `ON_ERROR_STOP=1` stops on SQL failure. [01-runtime-role.sh:app_password] [01-runtime-role.sh:ON_ERROR_STOP]
- This hook creates the role; migrations supply object grants. The hook is for first initialization, not password rotation of an existing volume. [01-runtime-role.sh:Creates the least-privilege role]

## Side Effects

- Runs psql against the selected database and creates a persistent PostgreSQL login role. [01-runtime-role.sh:psql] [01-runtime-role.sh:CREATE ROLE sagat_app]

## Verification

Run from the repository root:

```sh
bash scripts/smoke-compose.sh
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../../../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
