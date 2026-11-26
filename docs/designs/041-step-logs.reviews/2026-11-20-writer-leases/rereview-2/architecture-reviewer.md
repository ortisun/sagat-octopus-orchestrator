# Architecture re-read of decision D041-120 (design 041, writer leases, after the re-reads of fc4adff7)

Reviewer: `architecture-reviewer` subagent, read-only, on the snapshot of branch head `9361a112`. Saved verbatim.

Snapshot root: `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot`. File paths below are relative to it, and every line number refers to that snapshot. No git commands were run, so branches and worktrees were not checked again.

**Verdict.** No Blocker. There are 8 Majors:
- 4 must be fixed before approval: A, B, C and D.
- 2 must be fixed before the fourth run's first round: F and G. G is a setup defect that would stop every row.
- 1 must be fixed before O041-041 goes to the owner: E.
- 1 belongs in the same pass: H.

Minors and Nits follow.

## Status of the previous findings (rereads-fc4adff7)

| Finding | Status |
|---|---|
| M1 (the ensure had no error path or budget) | **Mostly closed.** ADR-0097 3b (:113-131), design :1196-1213 and data-model :263-272 now give the ensure a 200 ms budget, a tried fence, answers it returns instead of raising, and a store that never fails the report. Two things remain. ADR-0097 decision 4 still says the opposite (Major A). An ensure that raises anyway (`57014`, a guard it re-raises, a connection error) has no outcome label and no test (Minor 4). |
| M2 (the ensure is an unrecorded second transaction) | **Mostly closed.** Now done: ADR-0095 decisions 2 and 4, ADR-0097's supersede list with ADR-0084 decision 7's delay clause (:23-30), the ensure row of the budget table (:1116), the warm and cold equalities (:1126-1128), FR-041-005 (:161-163) and "What a report can wait for" (:904-917). Stale text remains in the Overview and elsewhere (Minor 7). |
| M3 (session affinity, the remembered room, idle reclaim) | **Partly closed.** Done: the 9-minute expiry, calls that return the pid, the session-affinity statement, G041-027 and a T041-9 pooler scenario. Not done: no pid-mismatch counter or fallback signal (Major H), and no task changes the runbook or the Helm advice (Minor 11). The 9-minute clock starts at the wrong event (Minor 3). |
| M4 (stale normative text) | **Closed.** ADR-0095 decision 5 now catches `55P03` and restates ADR-0027 item 1 (:89-101). Design :1029-1081, the T041-36 limit test (:2686-2691) and Constitution Check VIII are rewritten. |
| M5 (the fourth run's reading rules) | **Mostly closed.** Each gate has a consequence, `mark_only:busy` is now a recorded failure, and comparisons across builds are marked. Still open: the acceptance rule contradicts D041-115, and two consequences do not say whether T041-5 may start (Major F). |
| M6 (G041-025 closed while its residual stays) | **Closed.** G041-025 is restated and its closing condition aligned (research.md:161). G041-026 is a limit kept on purpose (:162), and O041-042 is registered (:215). A new evidence problem with O041-041 is Major E. |
| M7 (the fitness row) | **Closed.** The row names the tests (architecture-principles.md:31), and a `pg_locks` run-lock test (:2695-2697) and a 200 ms migration test (:2456-2457) were added. T041-5's list does not carry the new prototype tests over (Minor 9). |
| Contract change | **Mostly closed.** openapi.yaml:7636 and schema.d.ts:6080 now carry `tenant_quota_busy`. The domain enum does not (Major D). The description and `expiresAt` were not copied, and nothing checks for drift (Minor 14). |
| Earlier Minors | **Closed:** "one of the tenant's leases", G041-023, D041-117's withdrawal, ADR-0095 "up to three more", the Risks row, 26 nested statements, the commit queue, "a carry outgrew its plan", the function's return columns, the grants, the held-room figure, the marker label, the alert denominator, the section 9 lists, replica attribution, the Studio text, the revision header and the freeze budget (now a number, Minor 12). **Still open:** README:39, the metric owners (data-model:423-426), the lock-order test's writers (design :899-902). **Partly open:** the producer of the busy count (two producers now, Minor 5). |
| Nits | **Closed:** "5b.", "bound" in ADR-0095:115, the Studio count wording, `STABLE` in data-model:249 and step_log_spike.rs:304, and OD-2026-11-20-8/9 (no longer cited anywhere). One new remnant is in the Nits below. |

## Major

**Major A (must fix before approval). ADR-0097 decisions 4 and 8 still state pre-D041-120 rules that contradict decision 3b and the design.**
- **Where:**
  - ADR-0097:145-146: "The ensure (decision 3b) waits without that budget, under no run lock."
  - Against it: 3b (:121, "under the same budget as the fallback"), Consequences (:267, "waits at most the same budget") and data-model :270 (`bounded` true).
  - ADR-0097:182-190, decision 8, still lists `kind="refill"|"reclaim"|"idle"|"ensure"`, has no `step_log_quota_ensures_total`, and sets `caller` without `marker`. It says the alert fires "on any increase … over 5 minutes".
  - Against it: the design (:1312-1316, :1771-1772) and data-model :312-316 and :413-416 have `kind="ended"`, the ensures counter, `caller="marker"` and an alert at 0.1% of `step_log_carries_total` over ten minutes.
- **Consequence:** the owner is asked to accept (O041-042) an ADR whose decisions disagree with each other and with the design on the bound of the ensure, which is the core of D041-120, and on the operator's alert.
- **Fix:** rewrite :145-146 as "The ensure waits under the same budget, under no run lock". Replace decision 8 with the list in data-model §7 and the design's alert rule.

**Major B (must fix before approval). The claim "a report waits on the usage row at most once" is false.**
- **Where:** ADR-0097:123-125; design :1205-1207; D041-120 (1) (research.md:126).
- **What is wrong:** only `limit` and `held` are passed to the carry (data-model :509-510). An ensure that answers `busy` has already waited up to *b*. The report's reservation then fails, because the lease is still short, so the carry falls back and waits up to *b* again, this time under the run lock. T041-36's own case shows it: an "ensure whose usage row is held past its budget answers `busy` and the report lands" (:2693-2694). The row is still held, so that report waits about 2*b*, the second wait while holding the run row. A `made` ensure whose room a reclaim takes before the report also waits twice.
- **Consequence:** the bound that SC-041-007 and the Risks row rely on is understated. The case most likely to time out a second time is exactly the one that waits under the run lock.
- **Fix (pick one and record it):**
  - (a) Also pass `busy` to the carry, which answers `tenant_quota_busy` at once. There is then one wait and none under the run lock, at the cost of a few more busy losses. This is the architecturally stronger option.
  - (b) Restate the claim as "at most once under the run lock, at most two budgets in all".
  - Either way, add a T041-36 timing assertion for the busy-ensure case.

**Major C (must fix before approval). No port and no layer is named for the ensure, and its answer-to-mark-only rule contradicts ADR-0095 decision 3.**
- **Where:**
  - ADR-0095:72-74 says the worker and execution modules reach the step-log tables only through `StepLogCarryTx`, "whose one method makes that call and holds no rule". FR-041-008 (:196-197) says "one method, one call".
  - ADR-0097:123-125 and design :1205-1206 say "the store … passes a `limit` or `held` answer to the carry". That is a downgrade rule (Full → MarkOnly `tenant_quota`) placed in the adapter.
  - Design :980-989 makes `CarryPlan` an explicit argument chosen by the application.
  - The T041-36 build item (:2638-2645) is still the D041-119 text. It keys the room "per connection" and has no ensure answers, no `step_log_quota_ensures_total` and no handover to `CarryPlan`.
- **Consequence:** an implementer has to invent where the ensure lives. It needs the report's own connection before `begin`, which the application cannot see, while `CarryPlan` is decided in the application. Whichever side they choose breaks ADR-0095 decision 3 or puts a rule in the adapter. The T041-5/T041-36 source-text check only covers the log tables, so it would not notice.
- **Fix:**
  - Name the port. For example, the store's method that begins a carrying report takes `(tenant, planned)`, runs the ensure on the connection it acquired, and returns a closed `CarryRoom` enum (`Room|Made|Limit|Held|Busy|Contended|Invalid|Error`).
  - Have the application map `Limit|Held` (and `Busy`, if Major B (a) is taken) to `CarryPlan::MarkOnly(TenantQuota…)`.
  - Amend ADR-0095 decision 3 and FR-041-008 to name that method.
  - Rewrite T041-36's build item to match ADR-0097 3b.
  - Extend the source-text check so that only that method calls `sagat_quota_step_log_ensure`.

**Major D (must fix before approval). The built domain enum has no `tenant_quota_busy`, and no task adds it.**
- **Where:** `crates/domain/src/step_logs.rs:111-140`. `CeilingReason` is `AttemptRecords | AttemptBytes | TenantQuota`, and `parse("tenant_quota_busy")` returns `None`. The value exists in data-model :86 (the CHECK), contracts.md:233, openapi.yaml:7636 and schema.d.ts:6080. T041-4's list of revision changes (:2372-2374) and T041-36 do not mention the enum. The property test "each mark-only reason" (:2648-2650) compares with `plan_batch`, which cannot produce a busy reason.
- **Consequence:** a read path built on `CeilingReason::parse` returns `ceiling: null`, or an error, for a busy stream. The loss G041-026 is accepted on condition that it stays visible would become invisible to the reader, and nothing would fail.
- **Fix:** add a `TenantQuotaBusy` variant with `marker()` returning `None`, list it in T041-4, and state that the busy reason is produced only in SQL (outside the property test's comparison). Smallest fitness function: one test that parses the migration's `ceiling_reason` CHECK list and openapi.yaml's `StepLogStream.ceiling` enum and asserts that both equal `CeilingReason`'s `as_str` set.

**Major E (must fix before O041-041 is put to the owner). The owner would choose the busy policy on the rate of the regime where it is about zero.**
- **Where:**
  - O041-041 and G041-026 (research.md:214, :162) go to the owner "with task T041-35's fourth run's measured busy rate".
  - ADR-0097:8-9 says decision 3b "is measured by task T041-9".
  - The fourth run has no ensure before its reports. It leases at setup, keeps the tenant at the catalogue's maximum and uses direct connections (:2258-2268, :2283-2284).
  - The regimes where the loss is not rare (near the limit, behind a transaction pooler, a cold start) exist only in T041-9 (:2932-2955), which runs after acceptance.
- **Consequence:** options (a), (b) and (c) of O041-041, and the acceptance of ADR-0097 with G041-026 as its residual, would rest on evidence from the far-from-limit, session-affinity case only.
- **Fix:** state in O041-041 and O041-042 which regime the quoted rate covers. Declare now a threshold that reopens O041-041 (for example, busy carries above 0.1% in any T041-9 regime). Alternatively, ask O041-041 with T041-9's figures and record the delegate answer (a) until then.

**Major F (must fix before the fourth run's first round). The reading rules contradict the acceptance path of ADR-0095, and two consequences leave the next step open.**
- **Where:** design :2308-2309 says "Whatever the result, the coordinating session puts … ADR-0095 and questions O041-038 and O041-040 to the owner". Against it:
  - ADR-0095:5-6: put "once … T041-35 passes".
  - O041-038 (research.md:211): "Asked once T041-35 passes".
  - D041-115 (:126): "if the gate passes, ADR-0095".
  - ADR-0097's clause (:6-11) covers only ADR-0097 and O041-040.
- **Two consequences:** gate (2b) failing (:2297-2300) and gate (4) failing (:2302) name a lever and "reported", but not whether T041-5 may start. The base rule says "If a gate fails: stop; the design changes and is reviewed again before T041-5 starts" (:2186-2187). Gate (1) or (3) failing at 8 × 128 (:2301) does say it.
- **Fix:** keep "whatever the result" for ADR-0097, O041-040, O041-041 and O041-042, and keep ADR-0095 and O041-038 conditional on a pass, or record a new decision that changes D041-115. For (2b) and (4) at each size, state either "blocks T041-5 until …" or "goes to the owner with D041-109's environment question".

**Major G (must fix before the fourth run's first round). The declared setup cannot run: it asks a pool that is fully checked out for a connection.**
- **Where:** `tests/performance/scalability-ha/step_log_spike.rs:1758-1774`. `prewarm` holds every connection (`held.len() == max`, :1759-1764) and then runs `sqlx::query("SELECT spike_041.step_log_reclaim_idle($1)").execute(pool)` (:1770-1774). That call acquires from the same pool, which has no free connection. It waits for the acquire timeout (2 s, per the comment at :1750) and panics with "fold the leases of ended backends".
- **Second defect:** the `assert!(matches!(outcome, "made" | "room"))` at :1782-1785 panics if the setup ensure answers `busy` or `contended`.
- **Consequence:** with `PREWARM_LEASE` set, every row stops. A stopped round is void, so the fourth run is "not decided" again for a harness defect, and two hours of CI are spent on it.
- **Fix:** run the reclaim on `&mut *held[0]` (or before the fill). Retry `busy` and `contended` a bounded number of times in setup, and declare that in the run's text now. Fitness: a `leases_tests` case that calls `prewarm` with `PREWARM_LEASE` set on a pool of two connections and asserts that every backend holds a lease charged its share and the pool is full afterwards.

**Major H (same pass). There is no production signal for the regime D041-120's safety argument depends on.**
- **Where:**
  - `step_log_carries_total{outcome}` (data-model:405-406) has no path dimension.
  - `step_log_quota_refills_total{kind="refill"}` (data-model:413) mixes the ensure's refill, the carry's fallback, a batch's refill and the marker's.
  - No metric compares the pid returned by the ensure with the pid returned by the report's carry, although both calls return one (ADR-0097:125-127).
  - The only alert is on losses (`SagatStepLogRefillBudgetExceeded`, :1771).
- **Consequence:** "rare far from the limit and with session affinity" (ADR-0097:272-274) and G041-027 cannot be observed in production. An operator behind PgBouncer in transaction mode sees p99 grow, and fallbacks under the run lock, with nothing pointing at the cause until records are lost.
- **Fix:** have the store count the carry's path (for example `step_log_carries_total{outcome="stored_fallback"}` or `step_log_quota_fallbacks_total{caller}`), plus `step_log_quota_ensure_affinity_misses_total` when the report's pid differs from the ensure's. Add both to the runbook entry for G041-027.

## Minor

1. **C0 cannot run before the transaction.** ADR-0097:113, design :1196 and data-model :264 put the ensure "after the carry's checks C0 to C2". C0 runs "in the transaction, from what the report has already loaded under the run lock" (design :973-978), and the planned charge needs C3's sizes (:969-971). Fix: "after C1 to C3 and C0's role check". State that a managed step's carry, or a carry whose namespace is `off`, may cost an ensure whose room stays in an unlocked, reclaimable lease.
2. **The `invalid` ceiling is an attempt's maximum, not a carry's.** data-model :266-267 and step_log_spike.rs:387 use 16 MiB + 128 × 100,001 (about 29.6 MB). ADR-0097:118 says "larger than any valid carry's", and a carry is at most 64 records and 16 KiB encoded (:949). A defective planned charge can lease about 30 MB to one backend. Fix: derive the carry's bound, allowing for escaping and the marker, and use it in both places.
3. **The 9-minute clock should start at the lease's last charge or grant, not at the store's last sight.** An ensure answering `room` does not touch `charged_at`, so a room seen at minute 9 can be reclaimed at minute 10 while the store trusts it until minute 18 (ADR-0097:116, :125-127). Fix: the calls return `charged_at`, or the store timestamps the room only from calls that charged or granted. Add a test.
4. **An ensure that raises anyway is unaccounted.** The prototype re-raises other constraint names and `57014` (step_log_spike.rs, the ensure's `ELSE RAISE`). `step_log_quota_ensures_total` has no `error` value, and T041-36 (:2692-2697) has no case where the ensure raises. Fix: add `outcome="error"` and a test with an installed raising trigger in which the report's outcome, output and transaction count are unchanged.
5. **One metric has two producers, and one return value feeds two labels.** data-model :317-321 has the refill advance "one [sequence] per refusal cause", published by shard 0. :414-415 has the store count `step_log_quota_refusals_total{cause}` from the carry's cause. The ensure's caught refusals would also advance the sequences. `reclaim_idle` returns one integer for `kind="idle"` and `kind="ended"` (:281). Fix: one producer per metric, and a per-kind sequence or a two-column return.
6. **`idle_tenants` cost.** It scans the whole lease table with an anti-join on `pg_stat_activity` on every purge pass. The table has no index besides its key, and the comment at data-model :223-224 still says the reclaim "scans one tenant's rows by the key's prefix". State the cost against the lease-row gauge, and bound it.
7. **Stale text:**
   - Overview :19-28: "no transaction", "bounded by 200 ms".
   - :1009-1010: the function's returns.
   - :1021-1023: the mark-only reasons; no ensure answer.
   - :1029: "at most 200 ms".
   - :1032-1033: "the previous carry's returned room".
   - :889: "at most *b*".
   - FR-041-005 :162: "at most 200 ms".
   - ADR-0095:96: "at most four transactions" leaves out the ensure.
   - ADR-0095:150: "waited for at most 200 ms".
   - ADR-0097:84-85: should also say "and still alive".
   - ADR-0097:288: the list of functions.
   - Constitution Check VIII: gaps are listed only "to G041-024", and an "and" is missing before "one whose fallback".
   - `docs/adr/README.md:39`: leaves out the idle reclaim and the ingest and marker refills, and the new supersede entries.
8. **The lock-order test's writers** (design :899-902) still lack the ensure, the idle and ended reclaim, the limit-change trigger and the holders of `sagat_quota_lock_log_usage`. All are L4 holders that the deadlock argument (:881-893) relies on.
9. **The tests do not travel from prototype to build, and their mutations are not recorded.**
   - T041-5's creation list (:2417-2419) omits `reserve`, `counters` and `idle_tenants`.
   - Its test list (:2424-2434) omits the D041-120 prototype tests: `the_ensure_answers_without_raising`, `the_lease_of_an_ended_backend_is_reclaimed_at_once`, `a_carry_with_nothing_to_charge_opens_no_subtransaction` and `a_callers_lower_lock_timeout_is_kept`.
   - ADR-0097's "Enforced by" (:34-43) omits the `pg_locks` test and the 200 ms migration test.
   - The validation report records hand mutations only up to mutation 15 (validation-report.md:790-804), so none of the four new tests is shown to bite (rule 18).
10. **Nothing tests the cache expiry and the reuse of a pid together.** Covered by the test in Minor 3 if it includes a reused pid.
11. **G041-027 operations.**
    - No task changes `docs/runbooks/scaling.md:36` or `infra/helm/templates/_helpers.tpl:86`, which both advise "a pooler" without naming the mode.
    - The Risks table has no row for the pooler.
    - T041-9's PgBouncer scenario (:2950-2952) needs a new component with a pinned image, loopback-only ports and a healthcheck (AGENTS rule 16). T041-9's Files list does not name one.
12. **T041-9 criteria.**
    - "Under 1,000 a second (the rate below which `pg_subtrans` stays in its default buffers)" (:2946-2948) has no derivation. `pg_subtrans` pressure depends on overflowed subxid caches and the age of the oldest snapshot, not on a rate alone. Prefer direct measures: `pg_stat_slru` `blks_read` for `subtransaction` equal to 0 over the window, and no `SubtransSLRU` waits in `pg_stat_activity` samples.
    - The cold-start scenario (:2932-2938) lost its breach consequence ("reported to the owner", in the previous text). About 160 first ensures per tenant against p99 holds of 4.5 ms make `busy` answers predictable, so declare the expected figure and the consequence.
13. **G041-025's closing condition** ("twice the expected share", :2305-2307) has shares only for (a) and (h). State one for (f), or restrict the condition to (a) and (h).
14. **Contract copy and future additions.**
    - openapi.yaml:7636 lacks the `ceiling` description of contracts.md:233.
    - openapi.yaml:7639 still says "the last recorded batch"; contracts.md:236 says "batch or carry".
    - There is still no drift check between contracts.md §2 and openapi.yaml.
    - Adding to a closed enum is harmless now, because the schema is planned and unreleased. After release it would break strict clients (the Studio's TypeScript union, generated REST clients). Before the release, state that readers must accept unknown `ceiling` values, or record that the set is frozen.
15. **Metric owners** (data-model:423-426) still name no owner for the lease, ensure, refusal and budget counters, or for `caller="ingest"` and `caller="marker"`.

## Nit

- `main.rs:2352` and `:2586` take the threshold at index ⌊n × 99/100⌋. `percentile` (main.rs:301), which prints `carry_p99_ms`, uses ⌈n·p⌉ − 1. The "slower than the row's carry p99" count is therefore one element off the printed p99.
- The setup share's index `i` restarts in every pool (step_log_spike.rs:1775-1786), so the replicas share the same 20 fill levels and their refills come in correlated bursts. Seed `i` with the replica index, or say so.
- `without_the_refill_bound` still recreates the budget function as `IMMUTABLE` (step_log_leases_tests.rs:1085).
- The source-text check says "only the ensure … lock `tenant_retained_usage`" (ADR-0097:38, design :2437). The ensure takes the row through the refill.
- Behind a transaction pooler, the per-replica `application_name` of T041-5 is the pooler's, so a lease's backend can no longer be attributed to a replica.
- `lease_rows` (step_log_spike.rs, `run_row`) counts every tenant's leases, but the design says "the tenant's lease rows". This is harmless with one scenario tenant.

## Areas with no finding

- **Transactional boundaries.** The ensure is a transaction of its own that writes only quota state (the lease and G), under the usage row. I2 and I4 are kept, and room an ensure leaves unreserved is reclaimable. The report's transaction, its outbox and its engine state are untouched. Past the statement timeout the path is ADR-0095 decision 5's re-run without the carry, which is fenced as before.
- **The held-room bound.** The bound is now stated on unused room: each grant's share, plus planned charges reserved but not yet charged (ADR-0097:156-168, design :1255-1265). It follows from g = max(*n*, share) and from the reservation. The consecutive-grant floor is stated only for charges within their share, which is correct.
- **Idle and ended leases.** The reclaim is exact (S += `used`, G −= `granted`). A stale `pg_stat_activity` snapshot or a reused pid can cost a backend its room, never exactness.
- **The fourth run's arithmetic.** 9,000 × 3,280 = 29,520,000, and / 1,048,576 = 28.15 refills (0.31%). 9,000 × 64 × 328 = 188,928,000, and / 1,048,576 = 180.2 refills (2.0%). 2,654,435,761 mod 2^20 = 489,905, which is odd, so the shares are distinct and nonzero for small *i*. The expectation c / 1 MiB holds for a room spread uniformly over (0, 1 MiB]. No fourth-run round appears in validation-report.md, so the declaration does precede every round. Major G must still be fixed first.
- **Layers.** Apart from Majors C and D, everything here is SQL and adapter. No domain type takes I/O, a framework or `serde_json`.

## Files

- `$SNAP/docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md`
- `$SNAP/docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md`
- `$SNAP/docs/adr/README.md`
- `$SNAP/docs/designs/041-step-logs.md`
- `$SNAP/docs/engineering/architecture-principles.md`
- `$SNAP/specs/041-step-logs/data-model.md`
- `$SNAP/specs/041-step-logs/research.md`
- `$SNAP/specs/041-step-logs/contracts.md`
- `$SNAP/specs/041-step-logs/validation-report.md`
- `$SNAP/specs/001-platform-foundation/contracts/openapi.yaml`
- `$SNAP/frontend/apps/studio/src/api/schema.d.ts`
- `$SNAP/crates/domain/src/step_logs.rs`
- `$SNAP/tests/performance/scalability-ha/step_log_spike.rs`
- `$SNAP/tests/performance/scalability-ha/step_log_leases_tests.rs`
- `$SNAP/tests/performance/scalability-ha/main.rs`

`$SNAP` is the snapshot root given at the top.
