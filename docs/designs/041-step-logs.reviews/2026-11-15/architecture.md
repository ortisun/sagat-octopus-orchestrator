# Architecture review of design 041 (second round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 879c550b1726870fa56392b1cff20272f114a5ed
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer` agent was given the design path, ADR-0084, ADR-0085, research.md and its own report of 2026-11-13, and asked whether each first-round finding is answered and what new defects the revision has; it was not given the coordinator's conclusions or the EGM stage reports. Its report is recorded here, condensed; repository paths are relative to the root. New findings are numbered fresh for this round (B, M, N). The agent had no shell, so it could not run `git branch -a` or `git worktree list`; within the checkout only design 041 and G1618 of feature 020 mention step logs. Coordinator's note, not the reviewer's: the reviewer marks round-one M1 (the tail ends before a late batch) answered; EGM stage 5 finding 1 shows the end rule repeats the `xmax` defect feature 018 recorded in D1958, which the coordinator confirmed in `specs/018-schedules-bulk-operations/research.md` and `crates/infra-postgres/src/batch.rs`; see consolidated.md.

## First-round findings

| Id | Status | Where answered, or what is missing |
| --- | --- | --- |
| B1 | Partly answered | FR-041-008 (136-143), step 7 (492-499), Goals (255-256), SC-041-001, T041-6 spike, ADR-0084 decision 3, D041-013. Taking the row only at commit is the right fix; the wait bound and the hold at commit are wrong under concurrency (new M1). Open as a Major. |
| M1 | Answered | Step 1 (471-472), section 5 (594-597), D041-019, ADR-0084 decision 9, T041-12; a caveat on the horizon's scope is N8. |
| M2 | Answered | Lock order (514-519), section 7 (679-681), D041-020, T041-13; the ADR wording still contradicts it (N9). |
| M3 | Partly answered | Section 7 (674-680), D041-021, G041-007; the generation source is wrong inside collections (N1). Now a Nit. |
| M4 | Answered | FR-041-001 (91-92), section 6 (631), D041-014, ADR-0085 decision 2; checked against `host.py:172-178, 712`. |
| M5 | Answered | Section 1 (433-438), D041-022, T041-14; `worker/database.rs:22` authorizes `DatabaseExecute`, `worker.rs:1171-1173` offers managed nodes only to sessions that registered managed activities. |
| M6 | Answered | T041-5, T041-6, Verification Plan (1002-1005), D041-023; selector rule at `scripts/select-test-targets.py:71`; budget wording N2. |
| M7 | Answered | Section 9 (723-726), FR-041-018, D041-024. |
| M8 | Text lost | Not described in the first-round relay; not judged. |
| M9 | Answered | Overview (21-30), ADR status lines and README index agree with OD-2026-11-13-17; editing accepted ADRs in substance is new M3. |
| N1, N6, N8, N9, N10, N12 | Answered | Section 2 (452-466), SC-041-001, section 10, T041-5 (a slip remains, N3), FR-041-013 (fits `repeat_window`, `crates/domain/src/audit.rs:287`), FR-041-020. |
| N2, N3, N4, N5, N7, N11, N13 | Text lost | Not described in the relay; not judged and not counted. |

## Blocker

None.

## Major

1. What is left of B1: the tenant usage row is still taken by every batch, and the wait bound is wrong. FR-041-008 (140-142), ADR-0084 Consequences (105-107); ADR-0027, the hot-path gate. The finalizer is a `CONSTRAINT TRIGGER … DEFERRABLE INITIALLY DEFERRED FOR EACH ROW` (`20261024000000_durable_quotas.up.sql:446`; required by `tests/integration/security-multitenancy/durable_quotas.rs:370-380`), so a 500-line batch fires `sagat_quota_flush_deltas` about 502 times at commit; the first takes `tenant_retained_usage … FOR UPDATE` (`:348`) and holds it through the rest; each row also upserts `quota_transaction_deltas` (`:177-187`), which every completion writes. A completion with output waits behind every batch commit queued on the row, not "one log batch's commit"; the barrier test holds a single ingest. *Correction:* state the bound as the commits queued ahead, each holding the row for its remaining firings; add a T041-6 test with K concurrent ingests of one tenant (permits × replicas) recording completion p99; count the firings and the `quota_transaction_deltas` churn in T041-9.
2. The purge holds the same row for 5,000 trigger firings. Section 4 (566-571), FR-041-008. `PURGE_BATCH` is 5,000 (`crates/scheduler/src/passes.rs:76`); a delete with a negative delta still takes the row (`durable_quotas.up.sql:305-348`) and holds it through the batch; step logs can reach 100,000 rows per run. *Correction:* a step-log purge batch of at most 500 rows, the purge named in FR-041-008, and same-tenant completion p99 measured during a purge in T041-9.
3. Accepted ADRs were edited in substance, and the owner gate cites a decision made before the revision. `docs/adr/README.md:10` (an accepted ADR is not edited in substance), constitution Governance, ADR-0084 and ADR-0085 Status and Revision, design 319-320, T041-2 (840-843). Decisions 3 and 5-9 of ADR-0084 and 1, 3, 5, 6, 10, 11 of ADR-0085 were rewritten under "Accepted"; the owner accepted the text of `35b752b3`; T041-2's `Owner-gate: OD-2026-11-13-14` is already decided, so the gate is satisfied without the owner accepting the revision. *Correction:* status lines "Accepted (35b752b3, OD-2026-11-13-17); revision of 2026-11-13 proposed, not accepted", or superseding ADRs; point T041-2 and the slice-B tasks that build the rewritten decisions at the new OD row.
4. The `getScriptExecution` change is not additive, and the named check does not see it. Section 5 Contract (617-619), FR-041-022, constitution IV, "Contracts only grow" in `architecture-principles.md`. The response is `$ref: '#/components/schemas/ScriptExecution'` (`openapi.yaml:3645`) with `log` in `required` (`:7407`); the digest test hashes only path operations (`tests/contract/payload-classification/openapi.rs:346-362`), so making `log` optional changes no digest. *Correction:* keep `log` required and return `""` when withheld, plus an optional `logWithheld`; add a check that fails when a component schema used by an existing operation changes, or say none exists.
5. The classified-namespace backfill fails open and may implement the classification rule twice. FR-041-021 (201-204), T041-28 (905-908), principle V. A SQL search of stored documents would duplicate the walk in `crates/infra-schema/src/classification/walk.rs`; a post-deploy Rust job leaves a window in which a classified namespace reads as unclassified; `workflow_versions.classification_rules` is a rule version, not an "annotated" flag. *Correction:* run the backfill before the read routes serve, fail closed until a backfill-done marker exists, test that the backfill and the publication walk agree on the classification fixtures, and set the flag `WHERE NOT has_classified_schemas`.
6. The ingest's rules are placed in the adapter, and the port's shape is not stated. Section 2 (449-453, 468-512), T041-6 (864-867), principle II, `docs/architecture/overview.md:61-62`. T041-6 gives "`StepLogStore` and its adapter" the authority reads, deduplication, `DROPPED` folding, the three allowances, clamping and counters; today the binding and role checks are application code over a transaction port (`ensure_bound`, `ensure_activity_bound`, `crates/application/src/worker.rs:2715-2769`); a second copy in infra-postgres can drift on a security fence, and the folding rules could be tested only against PostgreSQL. *Correction:* a `StepLogTx` port that only reads and writes; a pure `plan_batch` in `crates/domain/src/step_logs.rs` with property tests; the application reuses `first_uncovered` and the role rule.
7. Rules of ADR-0084 that no check enforces. ADR-0084 "Enforced by" (12-21), `docs/adr/README.md:11`, AGENTS rule 21. Rule 1 (no transition reads a log), the heartbeat half of rule 2 and rule 10 (no log lines on `/runs/{runId}/stream`) have no check and are not marked review-only; `work_budget.rs` does not budget the heartbeat. *Correction:* a source-text test in the style of `crates/application/tests/managed_kind_sites.rs` that the worker and execution files never name `step_log`, `StepLogStore` or `run_log_usage`; `work_budget` asserts no statement in the lease and completion windows names a log table; a contract test that the graph stream emits only `graph`, `reconnect` and `end`; rows in `architecture-principles.md`.

## Nit

8. N1 (what is left of M3): `record_execution` picks the latest `step.leased` for (node, attempt) (`crates/infra-postgres/src/scripts.rs:1798-1802`), which inside a collection can be another iteration's generation; `RecordExecutionBody` has no instance or generation (`dto_scripts.rs:953-979`) and the insert does not write the instance columns (`scripts.rs:1856-1863`). The runner holds `lease.generation` and `lease.instance` (`apps/sandbox-runner/src/lib.rs:1057-1074`); add them to the report.
9. N2: SC-041-001 (216-217) says the budget passes "with a worker that logs on every step", but `work_budget.rs:32-53` drives the engine directly with no SDK worker; say the ingest runs through the store between the measuring windows.
10. N3: section 3 (548-551) creates triggers "for the three tables"; only `step_log_lines` has `log` columns (`durable_quotas.rs:345-349`); triggers on the others only add cost.
11. N4: stale facts: migrations 20270113 to 20270116 follow the anchor (523); the migration also needs the oracle update `scripts/check-upgrade-boundary.py` checks; `current_worker_authorized` is at `execution.rs:1017`.
12. N5: the namespace-scope open question (290-292) is settled by `QuotaScope::Namespace` (`crates/domain/src/quota.rs:112-117, 226-257`); the real question is how an on/off switch fits a catalogue of numeric limits where `Policy` is positive.
13. N6: admission uses one shared in-flight counter and sheds discovery above 3/4 (`crates/api-grpc/src/limits.rs:548-563, 595`); say whether `Class::Logs` counts in it and test that a full Logs class leaves discovery unchanged.
14. N7: the tail re-checks the grant every poll (611); `has_effective_payload_reveal_grant` takes the membership row `FOR SHARE` (`reveal_grants.rs:47`), assigning an xid and creating multixacts at up to 2 polls/s per stream; the graph stream re-checks at `limits.recheck` (`execution.rs:1381`).
15. N8: the horizon is held back by any transaction with an id in the whole cluster, prepared transactions included, not "anywhere in the database" (591); the held-transaction tests share a container with other test databases.
16. N9: ADR-0084 decision 11 (77-78) requires the stream row before the fences for every writer, but the script marker takes `take_log_prefix` first (`scripts.rs:1855`); reword or reorder.
17. N10: `DELETE /runs/{id}/logs` updates many stream rows in no fixed order (573); two deletes of one run can deadlock; lock with `ORDER BY stream_id FOR UPDATE`.
18. N11: the stream unique key has nullable `generation` and instance path for scripts (530-532); declare `NULLS NOT DISTINCT` or rely explicitly on `record_execution`'s advisory lock.
19. N12: "a test asserts every stream's run exists" (Risks 753, ADR-0084 109-110) is in no task.
20. N13: the session check (473-474) omits `worker_sessions.expires_at` and the match of the session's and the run's `namespace_id`.
21. N14: the capture floor must not depend on the host logger's enabled check (Python `isEnabledFor`, Go `Handler.Enabled`, C# `IsEnabled`); say so and set the host level above the capture level in `step-logs-capture`.
22. N15: section 9 does not add the new metrics to the telemetry metric enums and their tests; T041-25 omits `docs/architecture/overview.md` and the storage documentation.
23. N16: what `getScriptExecution` returns when `SAGAT_STEP_LOGS_READ_ENABLED=false` is not defined (584-585, 736).

Counts: Blocker 0, Major 7, Nit 16
