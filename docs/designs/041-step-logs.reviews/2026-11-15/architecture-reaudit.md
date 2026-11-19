# Architecture re-audit of design 041 (revision of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: b1b2c249 plus the uncommitted revision of 2026-11-15 (committed with this report)
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: one fresh `architecture-reviewer` agent was given the revised design, `specs/041-step-logs/{contracts,data-model,research}.md`, ADR-0084 and ADR-0085 with their Revision (2026-11-15) sections, and its own report `architecture.md`; it was not given `resolution.md`. It had no shell. Its report is recorded here, condensed; line numbers are those of the reviewed text. The author's answer to each open finding is in `resolution.md` section 5.

## Previous findings

M1, M2, M4, M6 answered; M3 partly (now Nit 2); M5 answered as asked, with a new defect in the mechanism chosen (Major 1); M7 answered, the test as written is wrong (Nit 4). N8-N11, N13, N14, N16-N22 answered; N12 partly (Nit 3); N15 partly (Nit 5); N23 not answered (Nit 6). The lock order was checked against the code (`execution.rs:4756-4759, 1027-1029`, `worker.rs:2729-2733`, `content_usage.rs:254-256`): no cycle found.

## Blocker

None.

## Major

1. The classified-namespace backfill relies on a guarantee the instance registry does not give, so the PII-grant rule fails open: a binary older than the registry does not register (`20270113000000_instance_registry.up.sql:21-22`, G1607 of feature 018), activation checks only registered instances (`:333-337`), and the registry ships in the same release. A classified version an old binary publishes after the backfill completes stays unrecorded. *Correction:* make the rule independent of the publisher (an insert-only record per checked version, or equivalent), or state G1607 as this feature's gap for the owner.

## Nit

2. Tasks that build revised decisions (T041-4, T041-10, T041-14, T041-28 to T041-30) have no "Waits for: O041-014"; T041-3 and T041-5 fix D041-030's contract without waiting for O041-019; O041-014 covers only the 2026-11-15 revision; option (a) should cite `docs/adr/README.md:10`.
3. data-model §6 misreads the quota catalogue: `validate` and `validate_ceiling` refuse 0 for every `Policy` resource (`quota.rs:547-549, 569`); the `EventRetentionSeconds` exception refuses 1-59; there is no `minimum()`.
4. The source-text test of T041-5 forbids `step_log` in the worker module, where T041-6 and T041-7 add `supports_step_log_protocol` and `STEP_LOG_PROTOCOL_MINOR`; forbid the table names and `StepLogTx` in the lease, heartbeat and completion code instead.
5. Horizon tests share one container (`tests/support/mod.rs:124, 227`) while the horizon is cluster-wide.
6. What `getScriptExecution` returns while `SAGAT_STEP_LOGS_READ_ENABLED=false` is not defined.
7. The classification analysis sits in a layer the publication store and the scheduler cannot call (`infra-postgres` does not depend on `infra-schema`; `validated_document_in` is private).
8. The selector misses the hot-path guard for `crates/application/src/step_logs.rs` and `crates/api-grpc/src/limits.rs`.
9. Admission does not protect the database pool: a quarter of a pool of 20 is 5 calls, and one tenant's 4 tickets take most of them.
10. A forged `end` in the plain cursor could make `complete` true for a running run.
11. Stale text: G041-003, ADR-0084 "Enforced by" and Consequences, "five resources" against "four `Policy` values".

Counts: Blocker 0, Major 1, Nit 10
