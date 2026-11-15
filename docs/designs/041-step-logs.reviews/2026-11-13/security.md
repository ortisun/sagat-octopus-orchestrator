# Security review of design 041 (first draft)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 2e90dbb854291a319ec94bc53b65ccef5fddcc0f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output was relayed by the coordinator in condensed form, grouped by subject across the three reviewers; this record lists the findings attributed to this reviewer, in the relay's words, with the answer of the revision. Findings the relay did not describe are listed as such and are not claimed answered.

## Blocker

1. B1. Python capture on the shared logger sees every attempt's lines and redacts them with the wrong secrets.
   *Answer:* capture in the per-attempt `_TaskLog` (`log`/`process`, raw message); every SDK redacts with the union of the secret values of all attempts in flight in the process; conformance case with two concurrent attempts each logging the other's secret (FR-041-001, FR-041-007, section 6, D041-014, SC-041-004, ADR-0085 decisions 1-2).

## Major

2. M1. The fence is the lease token alone: no `worker:execute`, role, binding, membership, expiry or run-state check.
   *Answer:* the ingest makes every report check short of the run lock (roles refused, `current_worker_authorized`, `bound_patterns`, `LEASED`/`RUNNING`, `lease_expires_at > clock_timestamp()`, run not finished or stopping); tests for binding removed, expired lease, database-runner token (section 2 steps 2-3, D041-015, T041-8, ADR-0084 decision 5).
3. M2. The tenant usage row held for the whole ingest (shared with architecture B1).
   *Answer:* as architecture B1 (D041-013).
4. M3. The server applies only one of the two existing scrubbers.
   *Answer:* both shape sets with one placeholder `[REDACTED]`, without `scrub`'s 2,048-character cut, in the application before sizing (FR-041-007, D041-016, ADR-0085 decision 3).
5. M4. SDK redaction misses keys, numbers, exceptions and language context.
   *Answer:* all covered by FR-041-007 and `sdks/SPEC.md`, tested by `step-logs-redaction` (T041-14).
6. M5. Not described in the coordinator's relay; not claimed answered here.
7. M6. Not described in the coordinator's relay; not claimed answered here.
8. M7. `DROPPED` markers trusted a worker's count.
   *Answer:* markers carry a sequence range, the server counts unseen sequences, saturating and clamped; no text; batches after `final` refused (FR-041-004, section 2 step 6, D041-018, ADR-0084 decision 7).
9. M8. No incident path to remove a logged secret; retention fixed at creation.
   *Answer:* admin-only, audited `DELETE /runs/{runId}/logs` (`run:logs:delete`, `run.logs_purged`); expiry computed at purge time (FR-041-023, section 4, D041-025, T041-30, ADR-0085 decision 9).

## Nit

10. N1. The 1.7 gate cannot be enforced: reports carry no session token. *Answer:* `session_token` added to the request; subject and minor checked (section 1, section 2 step 2).
11. N2. NUL and non-finite doubles. *Answer:* mapped (FR-041-003).
12. N3. Unsealed cursor; `stream` not run-scoped. *Answer:* sealed cursor bound to the run; `stream` matched with run and tenant (FR-041-010, section 5).
13. N4. The tail does not re-authorize. *Answer:* each poll re-authorizes through the reader, grant included (FR-041-012).
14. N5. The request could be logged. *Answer:* generated `Debug` skipped; never logged (section 1, FR-041-007).
15. N6. Studio rendering of bidi, zero-width and links. *Answer:* made visible, never auto-linked (section 8, T041-22).
16. N7. Sizing before scrubbing. *Answer:* scrub, cut, then size as `sagat_quota_measure` (D041-017).
17. N8. Admission and kill switch for reads. *Answer:* `[RunRead, RunLogsRead]`; `SAGAT_STEP_LOGS_READ_ENABLED` (section 5).
18. N9. Inconsistent status wording. *Answer:* as architecture M9; ADR-0085's status line repaired.

Counts: Blocker 1, Major 8, Nit 9
