# Correctness review of design 041, the third approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 6dceb5482d1a5a27b22be4018ed2df83e106137b
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, given the committed design, its companions, the ADRs, the constitution, the code and its own `correctness-approval-2.md`; no shell. Recorded condensed.

Previous findings: M1 to M3 fixed, M4 partly; N3 to N8 fixed, N1 and N2 hold. End rule, finalizer order, the one lock order, counting, duplicates and crashes checked and sound.

## Blocker

None.

## Major

1. A filtered read or tail declares the log complete after a 5,000-row scan that matched nothing, while matching lines are still unscanned: the end rule's "page is empty" no longer means "nothing left to scan" since D041-071. Define completion as the scan reaching the run's last row below the horizon; a 12,000-line test in T041-11 and T041-12.
2. T041-12's end-rule tests run on the shared per-process server and are not in the PostgreSQL 18 step, so they can pass with the defect they guard (another test's open transaction keeps the horizon low); run them on `isolated_server` and on 16 and 18.

## Nit

1. research.md's "Owner answers of 2026-11-15" still describes the old marker.
2. Hold points: no hold after T3; the 60 s lock timeout must be set at begin; every one of the K ingests needs the after-flush hold.
3. `tests/support/lock_order.rs`'s `Slot` has no step-log slots; name new ones.
4. The "largest id below end" case adds nothing; the "equals" wording is stale.
5. T041-13's terminal-transition test can pass either way; the hold must sit between steps 3 and 4.
6. T041-10's mark race cannot fail on a broken mark (T4 refuses an expired stream on its own).
7. `eventually`'s 5 s bound against 60 s holds; a skipped horizon test counts as a pass.
8. The work-budget claim is not built (no batch is stored in `work_budget.rs`).
9. Whether the tenant allowance counts the planned ceiling marker's 128 bytes.
10. The marker's lock variant names only the step-log accessor; the script log still needs `sagat_quota_lock_log_usage`.
11. `contracts.md:97`: `LEASE` "or the run is ending" against T2.
12. T041-7's p99 assertion is a timing test in a CI suite.

Counts: Blocker 0, Major 2, Nit 12
