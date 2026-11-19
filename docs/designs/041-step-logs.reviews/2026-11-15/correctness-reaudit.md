# Correctness re-audit of design 041 (revision of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: b1b2c249 plus the uncommitted revision of 2026-11-15 (committed with this report)
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: one fresh `correctness-reviewer` agent was given the revised design, `specs/041-step-logs/{contracts,data-model,research}.md`, ADR-0084 and ADR-0085 with their Revision (2026-11-15) sections, D1958 and `batch.rs`, the quota migration, and its own report `correctness.md`; it was not given `resolution.md`. Its report is recorded here, condensed. The author's answer to each open finding is in `resolution.md` section 5.

## Previous findings

r1-M3, r1-M5, M2, M3, M6, r1-N7, N1-N4, N7-N9 answered; M1 and M7 answered and owner-gated (O041-015, O041-017); M4 answered, its tenant branches broken (new M1); M5 answered for one purge transaction (new M3); M8 partly (Major 5); r1-N13 partly (Nit 13); N5, N6 moot. `workers.rs:328-344` checked: the session id is kept and the token hash replaced.

## Blocker

None.

## Major

1. The tenant-quota branches break the count rule: after the finalizer's raise the first transaction's gap count and `GREATEST` roll back, and the second transaction counts the kept records as dropped without re-planning, so a later final batch counts them again (or a gap is never counted); neither tenant branch sets `ceiling_sequence`; a resend between the two transactions can be counted as stored and dropped. *Fix:* the second transaction locks the stream and re-runs `plan_batch` with a tenant allowance of 0; a property test of the count rule.
2. The end horizon can be an id taken before the read saw the run finished: the read transaction's audit insert can assign its id first, and `pg_current_xact_id()` returns that id. *Fix:* REPEATABLE READ, or take `end` only when no id was assigned before the statement that saw the run finished; a test.
3. A purge that spans several transactions exposes half-purged streams (`purged_at` set only at the end) and lets a batch from an idle leased attempt in between. *Fix:* mark the stream in the first transaction; delete the rest through a partial index on marked streams.
4. SC-041-001's barrier tests cannot fail on what they claim: "its wait is recorded" is not an assertion, and held ingests time out at 200 ms and roll back. *Fix:* assert that all K committed and that the completion queued behind them (`pg_locks`), raise the test's lock timeout, give the bound a number.
5. r1-M8 still open: FR-041-008 says nothing of the per-run serialisation, and the run-usage row is now held through the authority locks and the commit.

## Nit

6. `records` counts the marker, so stored + dropped = highest sequence + 1 at a ceiling.
7. "A held transaction whose id equals `end`" cannot exist; restate as the largest ingest id below `end`.
8. A final batch answered `FINAL_ONLY` would send another empty final batch; treat it as the final batch's success.
9. `highest_sequence` has no bound; bound it at 2^62-1.
10. The unlocked tenant check must work out the limit as the finalizer does (`quota_limits`, `quota_ceilings`, 1 GiB, `log_enforcement_enabled`, `durable_quotas.up.sql:302-340`).
11. `quotas.rs:1324-1344` asserts that `share_secrets_lock` has one caller, inside `current_worker_authorized`.
12. The Verification Plan's lock-order row still names "an ingest near the tenant limit".
13. Line 3 of both ADRs still reads as in force.

Counts: Blocker 0, Major 5, Nit 8
