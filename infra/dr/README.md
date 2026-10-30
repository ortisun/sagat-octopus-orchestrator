# Disaster-recovery drill

## Overview

Disposable PostgreSQL point-in-time restore and primary-failover measurements for feature 010.

## Interface

- `drill.sh` accepts `PGBIN`, `SAGAT_DR_RUNS`, `SAGAT_DR_BASE_PORT`, report/keep settings and restore/failover/shard-takeover objectives. [drill.sh:find_pgbin] [drill.sh:SAGAT_DR_RUNS] [drill.sh:SAGAT_DR_REPORT] [drill.sh:SAGAT_DR_KEEP]
- `run_test` invokes exact `dr::` cases in the `perf_scalability_ha` Cargo target. [drill.sh:run_test]
- `journal_digest.sql` emits counts and ordered SHA-256 aggregates for runs, journal events and audit records. Projections are checked separately by replay. [journal_digest.sql:journal_sha256] [journal_digest.sql:audit_sha256]

## Invariants

- Restore must reproduce the recorded journal/audit digest, replay with `failed=0`, and finish within the configured restore objective. [drill.sh:identical] [drill.sh:restore_seconds]
- Synchronous failover requires positive acknowledged work, `sync` mode, `lost=0`, replay with `failed=0`, and promotion plus shard takeover within the configured objective. Measurements describe this drill environment. [drill.sh:sync_out] [drill.sh:promote_seconds]

## Side Effects

- Creates its own temporary PostgreSQL clusters, databases, WAL archives, backups, signals and logs; deliberately drops its primary schema and kills failover primaries. [drill.sh:make_cluster] [drill.sh:DROP SCHEMA] [drill.sh:kill -9]
- Writes the report selected by `SAGAT_DR_REPORT` and removes its own temporary directory unless `SAGAT_DR_KEEP=1`. The digest SQL only reads the selected database. [drill.sh:REPORT] [drill.sh:cleanup] [journal_digest.sql:SELECT]

## Verification

Run from the repository root:

```sh
infra/dr/drill.sh
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
