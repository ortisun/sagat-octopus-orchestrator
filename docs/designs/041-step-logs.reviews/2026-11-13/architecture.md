# Architecture review of design 041 (first draft)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 2e90dbb854291a319ec94bc53b65ccef5fddcc0f
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output was relayed by the coordinator in condensed form, grouped by subject across the three reviewers; this record lists the findings attributed to this reviewer, in the relay's words, with the answer of the revision. Findings the relay did not describe are listed as such and are not claimed answered.

## Blocker

1. B1. The tenant usage row is held for the whole ingest: `sagat_quota_lock_log_usage` takes `tenant_retained_usage … FOR UPDATE` to commit, and a completion with output needs the same row while holding the run lock, so completions queue behind log batches.
   *Answer:* the row is left to the commit-time finalizer unless usage is within 2 MiB of the limit, then taken as the last statement before the insert; `lock_timeout` 200 ms → `RETRY_LATER`; `Class::Logs` with a per-tenant sub-cap; barrier test; spike in T041-6 before T041-7; FR-041-008, Goals and ADR-0084 restated (section 2 step 7, D041-013, D041-027, SC-041-001, ADR-0084 decision 3).

## Major

2. M1. The tail can end before a batch that saw the lease live commits.
   *Answer:* ingest assigns its xid first; `end` only after `pg_snapshot_xmin` passes the `xmax` of the first terminal poll and a page is empty; held-transaction test (section 5, D041-019, T041-12, ADR-0084 decision 9).
3. M2. The script marker can deadlock with ingest (opposite lock order).
   *Answer:* one global lock order; the marker locks only its own `source = script` stream and does not charge `run_log_usage`; barrier test (section 2, section 7, D041-020, T041-13, ADR-0084 decision 11).
4. M3. `record_execution` has no generation or instance path; the marker's identity is not sourced.
   *Answer:* generation from the `step.leased` event it already reads; generation and instance path nullable for `source = script`; gap G041-007 (section 7, D041-021).
5. M4. Python capture as a handler on the worker-wide logger.
   *Answer:* capture inside `_TaskLog` from the raw message; never on a shared logger; FR-041-001, section 6, D041-014, T041-17, ADR-0085 decision 2.
6. M5. 1.7 implies 1.6 for ordinary SDKs, contradicting `sdks/SPEC.md` §7.1.
   *Answer:* §7.1 amended in T041-14; `ResolveDatabase` refused without `database:execute`, asserted by `step-logs-negotiation` (section 1, D041-022).
7. M6. The no-foreign-key and lock tests are in a target the hot-path selector does not pick; the `pg_locks` claim is not a test.
   *Answer:* tests moved to `tests/integration/claim-transition-efficiency/`; the lock-free test holds the run row and an `UPDATE` of the leased step under `lock_timeout = 50ms`; `pg_locks` claim dropped; rows in `architecture-principles.md` with the tests (T041-5, T041-6, D041-023).
8. M7. `SagatStepLogsDropping` alerts on customer behaviour.
   *Answer:* alerts on shedding and purge backlog only; ceiling drops shown per run (section 9, FR-041-018, D041-024).
9. M8. Not described in the coordinator's relay; not claimed answered here.
10. M9. Inconsistent status wording (Proposed mentions after acceptance).
   *Answer:* the design, ADR status lines and roadmap agree with OD-2026-11-13-17 and the revision.

## Nit

11. N1. Scrubbing belongs in the application, not the adapter. *Answer:* section 2, D041-016.
12. N6. SC-041-001 asserted a lock observation rather than a commit. *Answer:* reworded to commits under lock timeouts.
13. N8. Slice B stores lines before anything deletes them. *Answer:* capture defaults off until T041-10 (section 10).
14. N9. T041-5 must classify every column and create the generated triggers. *Answer:* T041-5 text and section 3.
15. N10. The audit collapse and filters. *Answer:* one record per subject and run per window whatever the filters (FR-041-013).
16. N12. The runbook must say policy changes are not retroactive. *Answer:* FR-041-020 and section 9 (switch and ceilings are not; retention is).
17. N2, N3, N4, N5, N7, N11, N13. Not described in the relay; not claimed answered.

Counts: Blocker 1, Major 9, Nit 13
