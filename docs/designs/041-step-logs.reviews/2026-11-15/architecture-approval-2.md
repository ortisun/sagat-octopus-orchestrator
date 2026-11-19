# Architecture review of design 041, the second approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fefcfafbb40d153ebd3e462c7ba2d94a7c0d9286
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, given the committed design, its companions, the ADRs, constitution 4.5.0, the architecture principles, the owner decisions and its own `architecture-approval.md`; no shell. Recorded condensed.

Previous findings: N1, N2, N4, N5 fixed; N3 holds in part (Nit 1 below).

## Blocker

None.

## Major

1. The ingest (step 3) and the script marker read the five step-log namespace policies without a policy fence, against ADR-0027 ("all authoritative limit reads occur after the corresponding policy fence"); the catalogue already takes `content_usage::policy_change` exclusively for non-Count resources (`crates/infra-postgres/src/quotas.rs:1020-1021`), so a lowered ceiling or capture off can be audited as in force while batches that read the old value commit after it. Read them under shared fences as `content_usage::admission_policy` does (`content_usage.rs:95-106`), before the stream row, and add the fences to the lock order; ADR-0084 decision 11's reading needs the owner's word.

## Nit

1. The selector still misses step-log code outside `step_logs` files (SDK capture points in existing files, the marker in `scripts.rs`, the `take_log_prefix` variant, the shared `follow`, the migration).
2. The hot-path source-text test sits in `crates/application/tests`, which never runs for an infra-postgres-only change; put the infra-postgres half in that crate.
3. The shared-secrets-lock test does not cover the new holder `step_logs.rs`.
4. `may_read` has no port for the script routes, which read with no transaction.
5. Two CI claims are wrong (`check-architecture.py` runs on a full run, `ci.yml:269`; the checkers are a local duty).
6. Down migrations: T041-4 needs a guard for rows of the new resources; T041-5's refusal on `classified_namespaces` blocks rollback forever.
7. ADR-0085's fitness functions name `mustNotLeak` and no check for "never printed by `Debug`".
8. 64-bit integer fields above 2^53 lose precision in the Studio.
9. A repeated delete "audits nothing" against ADR-0085 decision 9.

Counts: Blocker 0, Major 1, Nit 9
