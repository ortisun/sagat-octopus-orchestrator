# EGM stage 5 (initial zoom out) of design 041 (revision of 2026-11-13)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 879c550b1726870fa56392b1cff20272f114a5ed
Reviewer: egm-zoom-out (stage 5 of egm-review)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Target: the full file, lines 1-1047
Standard: `~/.claude/plugins/cache/egm/egm/0.1.0/resources/document_standards/zoom-out-standard.md`
GoldfishContext: docs/constitution.md; ADR-0084; ADR-0085; specs/041-step-logs/research.md; the three reports of docs/designs/041-step-logs.reviews/2026-11-13/
Note: one fresh `egm:egm-zoom-out` agent, dispatched directly (no synthesizer), preset "Consolidated report with all review rubrics". Its findings are recorded here in its own order, condensed; repository paths are relative to the root. The coordinator checked finding 1 against `specs/018-schedules-bulk-operations/research.md` (D1958) and `crates/infra-postgres/src/batch.rs:449-459, 652-657`: it holds.

Prerequisite checks of the standard: context sufficiency passes (Overview and The Problem, lines 9-82); root-problem focus passes; the design fits the constitution (PostgreSQL only, additive protocol minor, one conformance suite).

## Blocker

None.

## Major

1. The end-of-tail rule repeats a horizon check that feature 018 already proved wrong. Section 5 "End of the tail" (lines 594-597), FR-041-012 (lines 159-163), Risks (line 750), ADR-0084 decision 9.
   The tail records X = `pg_snapshot_xmax(pg_current_snapshot())` and sends `end` once `pg_snapshot_xmin >= X` and a page is empty. Feature 018 used this rule (D1033) and dropped it (D1958): a snapshot's `xmax` is the last completed id plus one, so a transaction still open with exactly that id passes; the held-transaction test showed "held 916, horizon 916, xmin 916". An ingest with that id that saw the lease live can still be open when `end` is sent, and its lines are never shown. The corrected, tested form is in `crates/infra-postgres/src/batch.rs:449-459, 652-657`.
   *Recommendation:* reuse D1958: horizon = `pg_current_xact_id()` taken after the run is seen finished, condition `xmin > horizon`; cite D1958 in section 5 and ADR-0084 decision 9; add the "held id equals horizon" case to T041-12.
2. The script-output marker adds a write path the read side could provide. FR-041-016 (lines 181-184), section 7 (lines 672-683), lock order (lines 514-519), section 3 `source` and nullable key (lines 529-532), T041-13 (lines 923-926), G041-007, Risks (line 751), Verification (line 1023).
   The marker needs a `source = script` stream written inside `record_execution`, nullable generation and instance path in the stream key, one more lock-order participant and a barrier test, only to point at a row that exists: `script_executions` has `(tenant_id, run_id, node, attempt)` and the unique index `script_executions_step_uniq` (`crates/infra-postgres/migrations/20261014000000_scripts.up.sql:250-278`). The marker is purged by `step_log_retention_days` while the execution lives by `script_retention_days`.
   *Recommendation:* drop the write; have `GET /logs/streams` also list the run's `script_executions` as `source: script` with their own expiry. Ask the owner only if the interleaved row in the whole-run tab is part of O041-009.
3. The `DROPPED` record kind repeats what sequence gaps already prove. FR-041-002, FR-041-015 (lines 94-98, 179-180), `StepLogRecord` (lines 414-418), section 2 validation and step 6 (lines 461-462, 488-491), section 6 (lines 644-645), Verification (line 1015).
   The worker numbers every record, dropped ones included, and one batch per attempt is in flight, so a gap between `last_sequence` and the next batch's first sequence is a drop the server can count without being told. The kind adds a record kind, two range fields, a reason enum, a validation rule, a fold step and a "lying ranges" test in six SDKs, the wire and the server.
   *Recommendation:* remove the kind; add the highest assigned sequence to the `final` batch so drops at the end are counted; the server counts gaps, may insert its own marker, and records `dropped_by_sdk`; keep the SDK-side reason in the SDK's local metric.
4. `StepLogPolicy` puts tuning numbers nobody asked for into the wire contract. Section 1 `RegisterWorkerResponse` (lines 423-425), FR-041-015 (lines 175-180), T041-3.
   The server already enforces every ceiling after redaction (FR-041-006) and `STOP_ATTEMPT` stops a worker that goes past one; buffer size and flush interval tuned from the server are speculative, and each field becomes a 1.7 contract six SDKs must honour and that cannot be removed (constitution IV).
   *Recommendation:* send only `enabled` (and the capture level if the namespace switch needs it); fix the SDK numbers as constants in `sdks/SPEC.md`.
5. The Technical Plan fixes schema, grants, metric names and UI internals in an approval-frozen document. Section 2 (lines 468-512), section 3 (lines 526-562), section 5 `q` predicate (line 599), section 8 (lines 689-706), section 9 (lines 713-721).
   After approval only checkboxes, evidence and follow-up tasks may change; column lists, grants, the exact `position(lower($q) …)` predicate, component names and metric labels make routine implementation changes force a new review. The load-bearing parts of section 2 (xid first, lock order, unlocked lease read, counters from inserted rows, scrub then cut then size) should stay.
   *Recommendation:* keep invariants and interfaces; move column lists, grants, metric names and Studio internals to `specs/041-step-logs/data-model.md` (as features 018 and 020 do) or into the task text.

## Nit

6. Three mechanisms for the tenant allowance (unlocked read, a hand-picked 2 MiB band, a retry after the commit-time raise). ADR-0027 requires a locking path, but the band is an optimisation decided before T041-6 measures it. FR-041-008 (lines 138-143), step 7 (lines 492-499), Complexity Tracking row 4 (line 778). *Recommendation:* drop the band; skip the insert when usage is at the limit; lock only after the raise; restore the band if T041-6 justifies it.
7. Two admission caps for one call: `Class::Logs` with its own cap, then a semaphore of a quarter of the pool plus a per-tenant sub-cap of 4. `Class::Transfer` owns one cap (`crates/api-grpc/src/limits.rs:494-496`). Section 1 (lines 444-445), section 2 "Permits" (lines 455-457), T041-7. *Recommendation:* one admission point with the per-tenant sub-cap inside it.
8. The read audit should reuse `AuditAction::collapses_repeats()` and `repeat_window()` (`crates/domain/src/audit.rs:259-302`); storing the first read's filter names in the record may defeat the collapse. FR-041-013 (lines 168-170), section 5 "Audit" (lines 603-606). *Recommendation:* add `RunLogsRead` to both lists; keep the record independent of the filters.
9. An open question a code read answers: `QuotaScope::Namespace` exists and `ScriptRetentionDays` uses it (`crates/domain/src/quota.rs:112-117, 226-257`); the catalogue's "zero disables" idiom (`quota.rs:886`) could express "capture off". Open Questions (lines 290-292), FR-041-020 (lines 197-200), T041-4. *Recommendation:* close the question; express capture off as an attempt-record ceiling of 0 at namespace scope.
10. The sealed cursor brings key management no other cursor needs (plain cursors in `crates/api-http/src/routes/resets.rs:43-53`, `routes/primitives.rs:193-207`), and the key's source is not named. FR-041-010 (lines 151-152), section 5 (lines 589-591). *Recommendation:* embed the run id in a plain cursor and refuse a mismatch, or name the key's source and rotation.
11. Each tail poll re-authorizes every 0.5 s, while the graph stream re-checks every `StreamLimits::recheck` (30 s, `crates/api-http/src/streams.rs:34-50`); the grant check locks the membership row (`reveal_grants.rs:40-67`). FR-041-012 (line 163), section 5 (lines 608-612). *Recommendation:* use the existing recheck interval unless the owner wants revocation to cut a tail within a second.
12. `run_log_usage FOR UPDATE` serialises every attempt of a run; a 200-iteration `forEach` (CUJ 6) can produce waves of `RETRY_LATER`; SC-041-007's scenario does not exercise the fan-out. Step 7 (lines 492-494), SC-041-007 (lines 236-240), T041-6, T041-9. *Recommendation:* add a wide fan-out case to the T041-6 spike.
13. `SAGAT_STEP_LOGS_ENABLED` defaults to false until the purge pass merges, then to true: a temporary rule caused by task order. Section 10 (lines 733-735), T041-7, T041-10. *Recommendation:* move T041-10 ahead of T041-7 and keep one default.

Verdict of the lens: not passing; well motivated and aimed at the root problem, but five Major findings are open.

Counts: Blocker 0, Major 5, Nit 8
