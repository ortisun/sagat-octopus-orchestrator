# Goldfish Review Report

**Target:** `docs/designs/041-step-logs.md` (`1-3214`, at commit `ff8033cd`; the revision since the approved commit `f9d4a65f`)

EGM review stage 5, Initial Zoom Out (simplicity and scope). Two independent instances of the zoom-out reviewer, A and B,
each read the whole target, the diff since `f9d4a65f`, ADR-0097, the relevant parts of `specs/041-step-logs/research.md`
and `docs/constitution.md`. Their findings are deduplicated below. The synthesizer checked the cited lines of every
Blocker and Major against the target before writing them down.

Both reviewers agree that the document's context is sufficient and that the root problem is the right one. T041-31
measured the per-step transaction, and carrying the last records on the report removes it. The findings are about weight
that the revision added on top of that.

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-zoom-out` (instance A) | completed |
| `egm-zoom-out` (instance B) | completed |

## Prioritized Findings (Blockers -> Major -> Nits)

No Blocker was reported.

### Z-1. The pre-report "ensure" is unmeasured machinery that brings back the transaction the redesign removed - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** Lines 25-28, 61-62, 171-178, 401, 929-933, 1145-1147, 1184-1185, 1227-1274, 1885, 1891-1892, 1947, 2327-2331, 2436, 2765-2779, 2826-2840, 3073-3101
* **Discrepancy Description**:
  * **What the ensure costs.** The revision's headline is that a short step needs no transaction of its own (lines
    61-62, 401, 1185, 1885). ADR-0097 decision 3b adds "+ 1, under no run lock" before a carrying report whenever the
    store's kept room is "short, unknown or stale" (line 1147). For a low-volume tenant that is nearly every report,
    because the room goes stale 9 minutes after the lease's last charge.
  * **What it brings.**
    * A per-(backend pid, tenant) room cache.
    * The eight-variant `CarryRoom`.
    * A dependency on session affinity, with gap G041-027: behind a transaction pooler the ensure does nothing (lines
      1264-1271).
    * Affinity-miss and ensures counters.
    * Warm and cold pins of the budget equalities (lines 1157-1159).
    * A second wait (lines 929-933).
    * A widened G041-021.
    * Two or three T041-9 regimes (cold start, pooler, fan-out) that exist only to judge it.
  * **What the evidence says.** The ensure has never been measured. ADR-0097's header says decision 3b "is measured by
    task T041-9". The one run of the lease design had no ensure before its reports (lines 2328-2331). It refilled
    inside the report, under the run lock, at about 0.31% of carries. It lost one carry in 864,000 to the busy reason
    (line 2436). At 8 x 128 every gate passed.
  * **A rejection that the data now contradicts.** Complexity Tracking (line 1947) rejects "no ensure, a bounded
    fallback only" because "every first charge and steady refill would wait under the run lock". The fourth run measured
    the steady-refill half of that and found it harmless. Only the cold start is left, and the cold start is unmeasured.
* **Rationale & Recommendation**:
  * Build the leases with the bounded fallback refill only: the 200 ms budget, then `tenant_quota_busy`.
  * Keep T041-9's cold-start regime and its criterion as the trigger for decision 3b.
  * If the regime fails, add the ensure as a contained follow-up in the store's begin method. A narrower form is also
    possible: ensure only when no lease is known for the (pid, tenant), with no tracking of short or stale room.
  * Correct the line 1947 rationale in any case.
  * Reopening decision 3b needs the owner, because ADR-0097 is accepted. The change stays within that one decision.
* **Recommended disposition: fix** — The only measurement contradicts the reason line 1947 gives for rejecting the simpler fallback-only design, and the ensure works against the revision's own headline goal.

### Z-2. The exact-lease machinery is heavy, and Complexity Tracking misstates the simpler alternative it rejected - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** Lines 46-54, 1200-1392 (section 2b), 1318-1332, 1433-1442, 1887, 1947, 2010-2012, 2043-2051, 2536-2568
* **Discrepancy Description**:
  * **The weight.** Section 2b is a small distributed allocator for a record that ADR-0084 calls lossy. It has:
    * a lease table and about twelve `SECURITY DEFINER` lease functions;
    * grants, reservations and refills with a lock budget;
    * near-limit, idle and ended-backend reclaim;
    * a limit-change trigger and a release branch;
    * seven quota counters, a gauge and two alerts;
    * a binomial-coefficient bound proof (lines 1318-1332).
  * **The gaps it leaves.** It also creates three residual gaps: G041-023, G041-026 and G041-027.
  * **What it buys.** D041-106 records that the usage-row queue "was not what failed the gates": with the charge on
    that row, the rate was 0.982 of the rate with no charge. The leases lift a ceiling of about 800 charging commits a
    second per tenant, and no FR or SC states that ceiling. Every completion that stores output still passes the same
    usage row (G041-024, line 1887), so the end-to-end ceiling of most tenants does not change.
  * **What the record says.**
    * ADR-0097 option 4 calls an unlocked ceiling, with an overshoot of at most "the pool size times a 24 KiB carry",
      admissible if proved and documented. It rejects that option only because quota views would carry a tolerance.
    * Complexity Tracking (line 1947) records it as "striped counters (false refusals or an overshoot)". It does not
      say the overshoot is bounded and admissible.
    * The row does not say that the leases leave the content-class ceiling unchanged.
    * This falls short of the constitution's Complexity Tracking rule, which asks the row to name the simpler
      alternative that was actually rejected.
* **Rationale & Recommendation**:
  * Keep the leases.
  * Rewrite the line 1947 row so that it names "one unlocked ceiling with a proved overshoot of at most the pool size
    times 24 KiB" as the rejected alternative. State the trade: a tolerance in quota views, against the lease machinery
    and three gaps.
  * Add that the leases do not change the content-class ceiling (G041-024).
* **Recommended disposition: accept** — The owner accepted ADR-0097 as written (OD-2026-11-20-15) and the fourth run measured it, so reopening the choice would cost another spike cycle; only the Complexity Tracking row needs correcting.

### Z-3. The `MarkOnly` re-run tier now serves only defect guards, and several places still describe the old quota re-run - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** Lines 21, 348-350, 402-403, 1016, 1049, 1114-1132, 1359-1360, 2784-2790, 3167
* **Discrepancy Description**:
  * **What the revision changed.** The carry function's block now catches the quota's own refusals in the same call:
    "The quota's own refusals never reach this path" (line 1124), and "no report is re-run for a quota refusal" (line
    1360).
  * **What is left of the tier.** The middle `MarkOnly` tier of the downgrade re-run now covers only a guard raise (an
    underflow or a lease `CHECK`, line 1121). Those are invariant violations, which means defects. The tier still
    carries:
    * `StoreError::StepLogRefused` and `AppError::StepLogRefused`;
    * the `quota_rerun` path;
    * the "run once more with none" sub-case (lines 1127-1128);
    * a four-transaction bound (lines 1117-1119).
  * **Text that still describes the old behaviour.**
    * SC-041-001 (lines 348-350) says a charge refused at the limit is "recorded by a downgrade re-run".
    * Line 1016 says "a mark-only carry after a step-log quota refusal".
    * Line 1118 enumerates "a step-log refusal, then `MarkOnly`".
    * The Verification Plan row (line 3167) says "A lease charge's raise at the tenant's step-log limit ... re-run[s]
      the report".
  * **A contradiction about counting.** Line 1049 says a re-run after a guard's raise records the reason
    `tenant_quota`. Line 1123 says those records are counted `failed`.
* **Rationale & Recommendation**:
  * Treat a guard raise like any failed carry call: one store error (`StepLogCarry`), one downgrade, `Full` to `None`.
  * Remove `StepLogRefused` and `quota_rerun`.
  * Restate the bound as at most three transactions: `Full`, the content fallback, then `None`.
  * Correct lines 348-350, 1016, 1049, 1118 and 3167, so that every place says quota refusals are answered inside the
    call.
* **Recommended disposition: fix** — A whole error tier now exists only to handle defects, and its stale descriptions in the success criteria and the Verification Plan would mislead the implementer of T041-36.

### Z-4. The write rules exist twice, once in Rust for the ingest and once in plpgsql for the carry - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A)
* **Affected Scope:** Lines 817-885 (ingest T1-T10), 1027-1054 (`sagat_step_log_carry` "implements exactly its T5 and T6"), 1086-1098, 1950
* **Discrepancy Description**:
  * OD-2026-11-20-1 moved the carry's plan into SQL. As a result, these rules now exist in two languages:
    * stream create and lock;
    * the counters;
    * the ceiling marker;
    * the order guard;
    * the batch plan.
  * Only a cross-language property test (D041-100) keeps the two copies equal.
  * Complexity Tracking (line 1950) records the rejected "plan kept in Rust" option, but not the double implementation
    that the choice causes.
* **Rationale & Recommendation**:
  * Have the ingest call the same database function in a waiting lock mode. Its T7 authority reads, which are unlocked
    anyway, would run before the call.
  * Keep `plan_batch` only as the test oracle it already is for the carry.
  * T041-6 is not built yet, so this is cheap now.
  * If the double implementation is kept, record it and the property test as its guard in the line 1950 row.
* **Recommended disposition: fix** — The ingest is unbuilt, so one write path for both transports costs little now, and the constitution's Complexity Tracking rule requires at least recording the duplication if it stays.

### Z-5. SC-041-007, the Goals, the Open Questions and T041-9 no longer agree with the accepted cost - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A; B reported the stale cost text as a Nit)
* **Affected Scope:** Lines 354-355, 374-386, 401-408, 438-452, 1884, 3071-3072
* **Discrepancy Description**:
  * **The criterion contradicts its own record.** SC-041-007 still requires "at least 90%" and "within 10%" (lines
    374-380), and the Goals repeat this (lines 405-408). The same criterion then records that the owner accepted 82.0%
    and +8.6 ms over the allowance at 1 x 16 (lines 380-386, OD-2026-11-20-13).
  * **T041-9 has no defined pass or fail.** T041-9 still says "a failure of SC-041-007 stops the release of the feature
    for a design change" (lines 3071-3072), and Open Questions says the same (lines 450-451). On the measured figures,
    T041-9 fails at 1 x 16 by construction. Its stop rule is therefore either void or would stop a release that the
    owner has already accepted.
  * **Stale cost text from the five-round-trip carry.** Since OD-2026-11-20-1, every carry is one statement (line
    1145, "+ 1"). Three places predate that:
    * Line 402 says "six for a long step".
    * Lines 447-448 say "round trips (five for a short step)".
    * Line 1884 states the p99 criterion as "within 10%", without the 2 ms allowance.
* **Rationale & Recommendation**:
  * Restate SC-041-007 per size: the accepted 1 x 16 figures as the floor, and the 90% rate and the larger of +10% and
    2 ms at 8 x 128.
  * Make T041-9's stop rule cite those per-size criteria.
  * Move the settled question out of Open Questions.
  * Correct lines 402, 447-448 and 1884.
* **Recommended disposition: fix** — The remaining measurement task, T041-9, has no well-defined pass or fail until the criterion matches what the owner accepted.

### Z-6. Finished spike protocols and revision changelogs hide the architecture, and the task record contradicts itself - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** Lines 24-30, 32-75 (four dated revision preambles in the Overview), 456-560 (answered owner questions repeated from `docs/owner-decisions.md`), 1149-1176 and 1383-1391 (statement-by-statement counts), 1318-1332 (bound proof), 2139-2188 (T041-31), 2189-2441 (T041-35's four runs)
* **Discrepancy Description**:
  * **How much is history.** The diff since `f9d4a65f` grows the design by about 1,300 lines. About 500 of its 3214
    lines are history, or protocol for spikes that are settled. Examples:
    * T041-35 keeps a hash seed, `((i + 1) x 2,654,435,761) mod 2^20`.
    * It keeps the derivation of the expected refill share and the judge script's metric names.
    * It keeps the reading rules for a run already decided (OD-2026-11-20-13, lines 2438-2441).
    * The same results are already in research.md (D041-116 to D041-122).
  * **The task record contradicts itself.**
    * Line 2440 says T041-35 "is ticked on that row with run 44834024933", but its checkbox at line 2189 is `[ ]`.
    * T041-31 (line 2139) is still an open checkbox, although its text says it is not run again.
* **Rationale & Recommendation**:
  * Cut T041-31 and T041-35 to their outcome, run id, owner decision and links to `specs/041-step-logs/validation-report.md`
    and `specs/041-step-logs/research.md`.
  * Tick T041-35 with its evidence, and withdraw T041-31 the way T041-28 and T041-34 were withdrawn.
  * Replace the four revision preambles with one pointer to research.md.
  * Move the bound proof and the nested-statement counts to `specs/041-step-logs/data-model.md` (section 3b or 8a) or to
    ADR-0097. Keep only the budget equalities in section 2a.
* **Recommended disposition: fix** — Implementers and later reviewers need the current structure, the moved text already lives in the linked records, and the unticked task contradicts the design's own evidence rule.

### `CarryRoom` has eight variants where the application acts on three - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out` (A)
* **Affected Scope:** Lines 1240-1246, 2770-2772
* **Discrepancy Description**: The application maps `Limit` and `Held` to mark-only and treats every other variant the same way. This finding goes away if Z-1 removes the ensure.
* **Rationale & Recommendation**: Return `Proceed` or `MarkOnly(reason)` to the application, and keep the eight outcomes only as the metric label.

### Broken sentence in the budget's third measure - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out` (B)
* **Affected Scope:** Lines 1159-1163
* **Discrepancy Description**: The text says "the test sets `pg_stat_statements.track = 'all'` for nothing:" and then says that the test support already starts PostgreSQL with that setting. The sentence contradicts itself and leaves a parenthesis unclosed.
* **Rationale & Recommendation**: Say only that the carrying cases assert `current_setting('pg_stat_statements.track') = 'all'` and fail with a message naming the setting when it is not.

### Two counters for one event - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out` (B)
* **Affected Scope:** Lines 1296-1298, 1373-1380, 1841-1843
* **Discrepancy Description**: `step_log_quota_refusals_total{cause="busy"}` and `step_log_quota_refill_budget_exceeded_total{caller="carry"}` count the same carry event, and `fallbacks_total{caller}` largely overlaps `refills_total{kind="refill"}`.
* **Rationale & Recommendation**: Keep one counter per event, and derive the rest in the alert expressions.

Counts: Blocker 0, Major 6, Nit 3
