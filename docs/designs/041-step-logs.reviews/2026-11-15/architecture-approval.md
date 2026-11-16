# Architecture review of design 041, the approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 882e56fcd513fa4d0de4205b95532113522f223a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, given the committed design, its companions, the ADRs, constitution 4.5.0 and its own `architecture-final.md`; no shell. Recorded condensed.

All eleven findings of `architecture-final.md` answered (M1 by OD-2026-11-15-16; M2 to M3 and N1 to N8 in the text).

## Blocker

None.

## Major

None.

## Nit

1. "No `SECURITY DEFINER`" (design 671-672, 1097) is stale against the two accessors of the ADR-0027 Amendment.
2. A fifth argument of `sagat_quota_add_delta` with a default, done with `CREATE OR REPLACE`, creates an overload and makes the existing calls ambiguous; drop and recreate, and the down migration restores the three functions.
3. T041-32's path rule cannot match handlers inside shared files (`routes/execution.rs`, `api-grpc/src/service.rs`).
4. The `recordScriptExecution` change has no clear owner (schema or operation description; T041-3 publishes before T041-13 accepts the fields).
5. An older replica fails quota listings once a step-log quota is set (`quotas.rs:1302-1308`); say so in section 10.

Counts: Blocker 0, Major 0, Nit 5
