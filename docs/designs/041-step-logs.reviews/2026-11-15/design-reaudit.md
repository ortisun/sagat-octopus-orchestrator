# EGM design-critic re-audit of design 041 (revision of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: b1b2c249 plus the uncommitted revision of 2026-11-15 (committed with this report)
Reviewer: egm-design
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Target: FeatureRange, lines 1-998
Note: one fresh `egm:egm-design` agent, the one re-audit the EGM cap allows; given the design, `specs/041-step-logs/{contracts,data-model,research}.md`, ADR-0027, ADR-0084, ADR-0085, D1958 and the previous `critic.md`, not `resolution.md`; it applied the persona's own rubric. Its report is recorded here, condensed. The author's answer to each open finding is in `resolution.md` section 5.

## Earlier design findings

Answered: 1, 2, 5 (escalated as O041-016; Nit N2 on its mechanism), 6, 8, 10-13, 21-23, 25, 26 (moot), 7 (as an accepted gap). Partly: 3 (the membership `FOR SHARE` MultiXact cost missing from Risks and from the spike), 4 (stream and usage rows kept until feature 020 with no growth estimate), 9 (Constitution Check V still unqualified), 24 (no cost bound on redaction with 1,024 values), 27 (no route removes a script execution's log; the runbook promises "how").

## Blocker

None.

## Major

- A. The end horizon can come from an id the read transaction took before it saw the run finished (the audit insert assigns one), so `complete` can be true while an ingest that saw the lease live is open; G1957 of feature 018 again. *Fix:* take `end` in a transaction with no id assigned before the observation.
- B. "Every missing line is accounted for" is false for stops at session or process level: with no stream the Studio shows "no log". *Fix:* send the empty final batch where the server is reachable (secret set full); otherwise narrow the Goal and SC-041-003 and register a gap.
- C. A retention applied at purge time contradicts ADR-0027 ("does not rewrite stored expiry … or authorize retroactive deletion", line 93), which ADR-0084 claims to apply unchanged.
- D. The fail-closed classification breaks script logs for every role (no role implies a reveal grant, `reveal_grants.rs:36-39, 54-56`) and hides step logs by default until an operator activates the capability and the backfill completes; only viewers are named.
- E. The Logs class shares the database pool with lease calls (a quarter of a pool of 10 or 20); with the defaults the per-tenant tickets can never all be used, and K and the 4 × R bound do not occur.

## Nit

- N1. Step 3 does not say how it reads the session (`worker_session_room` locks it `FOR UPDATE`) or whether the policy reads take ADR-0027's fences.
- N2. The `fixed_octets` charge is on `step_log_lines.sequence`, a `bigint`, which the manifest test does not compare (`durable_quotas.rs:323-343`); use a `NOT NULL` text column.
- N3. `records` includes markers, so the count rule is off by one.
- N4. `last_at`'s clock is unstated; a stream with no stored line never expires.
- N5. The second transaction of step 10 can double-count; it must re-plan under the stream lock.
- N6. Fact 3's budgets are "with a next step" and "that ends the run", not with and without output (`work_budget.rs:224-225`).
- N7. G041-003 and G041-007 still describe the removed marker; ADR-0084 Evidence cites the spike of T041-6.
- N8. Each line upserts the transaction's `quota_transaction_deltas` row, which bloats while the horizon is held; the spike should include a held transaction.

Counts: Blocker 0, Major 8, Nit 10
