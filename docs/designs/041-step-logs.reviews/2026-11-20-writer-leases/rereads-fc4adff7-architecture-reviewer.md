# Architecture re-read of decision D041-119 (writer leases, the ensure, the 200 ms fallback)

Reviewer: `architecture-reviewer` subagent, read-only, on `fc4adff7`. Saved verbatim.

No Blocker remains. Your Blocker B1 is closed. There are 7 Majors: 3 must be fixed before approval, 1 before the fourth run's first round, and 3 in the same pass. Minors and Nits follow.

Files: worktree `/Users/marcelo/Documents/Projects/sagat-wt-041`, commit fc4adff7. Read-only tools only. No git commands were run, so no branches were checked again.

## Status of the previous findings

| Finding | Status |
|---|---|
| B1 | **Closed.** Past *b* the carry's block answers `tenant_quota_busy` in the same call and the report commits (ADR-0097:116-120, data-model.md:549-562). Two gaps remain around it: the ensure's error path (Major 1) and stale text (Major 4). |
| M1, M2, M3, M5, M6 | Closed. |
| M4 | Closed: `caller` label, counted by the store, alert on carries only. |
| M7 | Closed in ADR-0097, data model §3b, the design's lock-order text and the fitness row. One stale paragraph is left (Major 4). |
| M8 | Mostly closed: the force is restated, there is a Risks row and a T041-9 fan-out scenario. "What a report can wait for" leaves out the ensure (Major 2). |
| M9 | Partly closed (Major 5). |
| Minors | Most taken. Still open: the "one of the tenant's leases" text, 23 against 26 nested statements, the section 9 metric list, no O-question for accepting ADR-0097, the lock-order test's set of writers. |
| Nits | Still open: "5b." |

## Major

**Major 1 (must fix before approval). The ensure has no error path and no budget, and a naive build re-opens B1.**
- **Where:** ADR-0097:100-108; docs/designs/041-step-logs.md:1181-1188 and :2565-2572 (T041-36); specs/041-step-logs/data-model.md:261-264.
- **What is wrong:** The ensure calls the refill, which raises `tenant_step_log_quota_exceeded` when g > F. At the tenant's limit every ensure raises. Nothing says the store swallows that error. Propagated with `?` in the report path, it fails the report at the limit, which is the retry-exhaustion path B1 was about. The ensure waits "as the store's statement timeout allows" (5 s, or 3 s), holding the step-log fence shared. A limit change that queues for that fence exclusively makes every carry of the tenant fail its `pg_try_advisory_xact_lock_shared` (new requests conflict with the queue). So all of the tenant's carries are `contended` for up to the statement timeout. At the limit each carrying report pays two waits on the usage row: the ensure, then the fallback. The ensure's waits and timeouts are not counted. `kind="ensure"` counts calls only.
- **Fix:** State that the ensure is best effort: its own budget (for example *b*, or a fixed 1 s), and every error (the quota refusals, `55P03`, `57014`) swallowed and counted (for example `caller="ensure"` on the budget counter, or `step_log_quota_ensures_total{outcome}`). The report then proceeds. Optionally have the store remember "tenant at limit" for the connection, to skip the second wait. Add T041-36 tests: an ensure at the limit and an ensure past its budget each leave the report's outcome, output and transaction count unchanged.

**Major 2 (must fix before approval). The ensure is a second transaction on the report path, and the ADRs, budget and bound do not say so.**
- **ADR-0095 decision 2:** "A carrying report MUST add no transaction and exactly one statement" (ADR-0095:62-63). The ensure adds one transaction whenever the lease is short, including a fresh connection's first carry.
- **Budget table:** "What a report pays" (design :1101-1105) has no ensure row. The `work_budget.rs` equalities (51/1/11) will not hold for a cold connection unless the test warms the lease first, and the design does not say which.
- **The bound:** "What a report can wait for" (design :896-918) and the Overview (:24-28) list only the fallback refill. FR-041-005 (:153-154) allows the ensure's time with no bound.
- **Supersede list:** ADR-0097:22-23 says it changes "nothing" of ADR-0095 "except where its charge goes", yet decisions 3b and 4 add a transaction, a wait and a new answer. ADR-0097:63-64 cites ADR-0084 decision 7 as "never fail, block or retry" and leaves out its "or delay a step beyond one bounded final flush" (ADR-0084:172). The ensure and the fallback are step-log delays of a report.
- **Fix:** Add an ensure row to the table: +1 transaction and its statements, under no run lock. Pin a warm and a cold equality in T041-36. Name the ensure in ADR-0095 decisions 2 and 4, the Overview and the "What a report can wait for" paragraph. Add ADR-0084 decision 7's delay clause, and ADR-0095 decisions 2, 4 and 5, to ADR-0097's "Refines / supersedes".

**Major 3. The room the store remembers per connection is often not the room of that backend's lease, so the ensure silently becomes the fallback under the run lock.**
- **Where:** ADR-0097:102-104 ("on the same connection, so on the same backend") against ADR-0097:71-72 and design :1171 ("behind a transaction pooler too"); design :1243-1244; T041-36 :2565-2568.
- **Three cases:** Behind a transaction pooler, two transactions on one client connection can land on different server backends. The Helm chart (`infra/helm/templates/_helpers.tpl:86`) and `docs/runbooks/scaling.md:36` both tell operators to put a pooler in front of the server. After the idle reclaim deletes a lease (10 minutes without a charge), the store still remembers its room, so it skips the ensure. The reservation then fails and the report falls back. That is exactly the "many low-volume tenants" regime. The claim at :1243-1244 that such a writer "pays one ensure to lease again" is false. An ingest or a script marker on the same backend spends the lease's room behind the store's back.
- **Consequence:** fallbacks become the common path in these regimes, so waits under the run lock, subtransactions and `tenant_quota_busy` answers all go up.
- **Fix:** Expire the remembered room at the idle interval, minus a margin. Have the ensure and the carry return `pg_backend_pid()`, and count mismatches. State in ADR-0097 3b, its Consequences and the runbook that the ensure helps only with session affinity (direct connections or a session pooler). Have T041-9 measure through a transaction pooler, or register the gap.

**Major 4 (must fix before approval). Normative text still describes rejected mechanisms.**
- **Design :907-913:** a carrying report "tries … on a refill, L4, and when … the refill's L4 is held (… a refill in progress), writes nothing more (`contended`)". This is the `NOWAIT` refill. It contradicts :897-900 and section 2b.
- **T041-36 test (design :2613-2617):** the refused carry is "re-run with a mark-only carry". Section 2b (:1263) says "no report is re-run for a quota refusal". The test also says "the carry sets no lock timeout", which is now only partly true.
- **ADR-0095 decision 5 (:86-87):** the block "catches only those two refusals (by constraint name), re-raising anything else". It must also catch `55P03`, which has no constraint name (data-model.md:554-557 does). Design :1064-1065 repeats the stale rule.
- **ADR-0095 decision 5 (:95-96):** it restates ADR-0027's Amendment item 1 as "recorded by a re-run without them". A refusal is now answered in the same call; only an uncaught guard leads to a re-run. This is the wording the owner is asked to accept (O041-038).
- **Constitution Check (design :1825):** a carry "whose refill finds the usage row held (`contended`)".
- **Fix:** rewrite each of these to the waiting, budgeted refill and the in-call answer.

**Major 5 (must fix before the fourth run's first round). The fourth run's reading rules are incomplete and contradict ADR-0097.**
- **Where:** design :2201-2239.
- **Gaps in the rules:** The expected outcome has no single reading. Rule (iii) covers only (1) and (3) at 1 × 16. Rule (ii) covers (2b) only at 8 × 128. A failure of (2b) at 1 × 16, which :2229 expects, and a failure of gate (4) fall back to the first run's "stop; the design changes" (:2138-2139). So two 1 × 16 failures lead to different consequences. `mark_only:busy` still only voids rounds, so a systematic busy rate leaves the run "not decided, may be run once more", as in the third run. The run cannot fail on the failure mode it introduces; that part of M9 is still open. Rule (iv) puts ADR-0097 to the owner only on a full pass with (i) closed. ADR-0097:6-10 and O041-040 (:431-433) say "once the fourth run is recorded, whatever that run's result". The expected figures (:2223-2230) compare with medians of the second and third runs, taken on the build before `5458820b`, and do not say so.
- **What is clear:** the setup addition and the build measured are stated clearly.
- **Fix:** Give each gate at each size one declared consequence. Make a round that would be void only because of `mark_only:busy` a recorded failure of the refill design rather than a re-run. Align rule (iv) with ADR-0097's acceptance clause. Mark the expectations as comparisons across builds.

**Major 6. G041-025 would be "closed" while D041-119 deliberately keeps its residual.**
- **Where:** specs/041-step-logs/research.md:160; design :1782 and :2231-2232.
- **What is wrong:** G041-025 records a loss for no quota reason "which the owner will not accept". D041-119 accepts exactly that loss past *b*, as a delegate decision under OD-2026-11-20-6, which requires reporting to the owner afterwards. A zero count in a spike lasting minutes would close a gap that the design keeps on purpose. The close condition ("no unwritten carry", which includes `contended`) differs from rule (i) of the fourth run (`mark_only:*`). There is still no O-question for accepting ADR-0097. Only T041-5 (:2394) mentions that acceptance.
- **Fix:** Restate G041-025 as "Open as a limit: bounded, visible, alerted; accepted by delegate (OD-2026-11-20-6); reported to the owner". The fourth run then measures its rate instead of closing it. Register an O-question for accepting ADR-0097 that names this residual.

**Major 7. The fitness row does not cover the rules this revision relies on.**
- **Where:** docs/engineering/architecture-principles.md:31.
- **What is wrong:** The row states "a fallback refill inside a report waits for the row at most 200 ms", but its checks name only the held-lock, deadlock, invariant and property tests and the source-text check. No check enforces "the ensure holds no run lock", which is the core of D041-119. No check enforces the production value of the budget function (`sagat_quota_step_log_refill_wait()`, data-model.md:246), which a test database may replace.
- **Smallest fix:** Name `a_refill_past_its_budget_answers_busy_and_the_report_lands` and `a_refill_restores_the_callers_lock_timeout` in the row. Add a T041-36 test: with the usage row held and a cold lease, `pg_locks` shows that the waiting ensure's backend holds no lock on the run's `runs` row and no transaction id of the report, and the report commits after release. Add a migration test that a freshly migrated database returns `interval '200 milliseconds'`.

## Contract change

Not handled the way the platform handles contract changes, and the design does not say it must be.
- `contracts.md:233`, data-model.md §1 and §2 (the CHECK constraint) and section 8's Studio text all carry `tenant_quota_busy`.
- The copy already merged by T041-3 (built but not ticked, design :2076) does not: `specs/001-platform-foundation/contracts/openapi.yaml:7636` still has `enum: [attempt_records, attempt_bytes, tenant_quota, null]`, and :7639 still says "last recorded batch" with no carry. `frontend/apps/studio/src/api/schema.d.ts:6080` matches that stale copy. `docs/reference/api.md` lists operations only, so it is unaffected.
- The change is additive to an unreleased planned schema, so there is no compatibility risk.
- T041-3 (:2272-2279) says "copy §2", but waits for O041-038 and does not say the merged copy is stale. Under OD-2026-11-20-8, the planned copy is a document.
- The operations digest test (`tests/contract/payload-classification/openapi.rs:343-394`) excludes everything planned for 041 and hashes no schemas, so nothing catches drift between contracts.md and openapi.yaml.
- **Fix (Minor, rule 10):** Update openapi.yaml now, run `npm run api` and `npm run check:api`, or add a correction line to T041-3. Add the smallest check: a contract test that parses the YAML blocks of contracts.md §2 and compares them with the matching `components.schemas` in openapi.yaml.

## Minor

**Leftover stale text**
- **"One of the tenant's leases":** design :190 (FR-041-008), design :1060, ADR-0095:65. Write "its writer's own lease".
- **G041-023** (research.md:158) still says "past its bound fails the report, which is re-sent", and that a grant is "n − room for a larger charge".
- **D041-117**'s correction note (research.md:128) does not withdraw "a carry whose tenant has room always writes".
- **ADR-0095:149** says "one or two more report transactions"; decision 5 allows up to three more.
- **`docs/adr/README.md:39`** leaves out the idle reclaim and the ingest's refill among the takers of the usage row.
- **Risks row (design :1778)** still points at the third run.
- **Statement count:** design :1121-1123 says 23 nested statements, with no reservation; :1285-1287 and data-model.md:314 say 26.
- **Commit queue (design :903-905):** the list of what a report queues behind at its commit leaves out ensures and other carries' fallback refills.

**Contradictions inside the design**
- **"A carry outgrew its plan"** is listed as a fallback (ADR-0097:108, design :1190, D041-119), but ADR-0097:98 says the plan MUST bound the charge. On the reserved path there is no block, so such a refill's `55P03` or refusal escapes to a re-run, and design :1083 says that cannot happen. Drop the case: it is a defect that the property test catches.
- **Function return:** data-model.md:499-502 `RETURNS TABLE` has no cause and no room column, but :561-562 and T041-36 (:2568) rely on both. The comments at :498 and :500-501 leave out `tenant_quota_busy`.
- **Grants:** data-model.md:339-340 grants `EXECUTE` on `ensure` and `reclaim_idle` to `sagat_app`. T041-5's test (design :2383-2384) allows only allowance, reserve, usage and counters.
- **Held-room figure:** design :1234-1235 quotes "about F₀ / 20" with 128 writers in the general-case sentence, but that is the bound for consecutive grants only.

**Operability**
- **No producer for the busy refusal count:** `step_log_quota_refusals_total{cause="busy"}` has none. The refill raises before it could advance a sequence (data-model.md:304-306). Say the carry's handler or the store counts it.
- **The marker has no caller label:** a script marker's refill past 200 ms falls outside the `caller` label.
- **Undefined denominator:** `SagatStepLogQuotaHeldRefusals` is "above 1% of charges" (design :1727), but no charges metric exists.
- **Lists out of date:** Section 9 (:1732-1733) lists three lease metrics instead of five. Constitution Check VI (:1823) leaves out `SagatStepLogRefillBudgetExceeded`. Complexity Tracking (:1836) says "an alert"; there are two. The metric owners in data-model.md:409-412 do not name the owner of the lease counters, the counters accessor or `caller="ingest"`, which is ingest code that T041-36 changes.
- **Replica attribution:** the per-replica `application_name` (T041-5) is overwritten if the built function keeps the prototype's "application name" statement (design :1121). State that the built function sets none.
- **Studio text:** "the tenant's log quota is full" (design :1714) is wrong for `tenant_quota` that comes from held room; the contract says "full, or its last room held".

**Tests and records**
- The lock-order test's set of writers (design :891-894) still has no holder of `sagat_quota_lock_log_usage`, although section 2b now argues about them.
- The T041-9 low-volume criterion (:2863) depends on a "freeze budget stated in the runbook" that does not exist yet, so it is not fixed before the run.
- The design's revision header (:30-68) does not record the writer-lease revision, its review folder or ADR-0097.

## Nit

- ADR-0097:144 still numbers a decision "5b.", and the idle reclaim is stated twice (decision 1 :73-76 and 5b).
- ADR-0095:110 says "bound"; the term is now "budget".
- Design :1714-1715 says "with the count dropped" and then "each with its drop count".
- `sagat_quota_step_log_refill_wait()` is `IMMUTABLE` (data-model.md:247). A test database that replaces it can meet plans that already folded the old value; `STABLE` is safer.
- OD-2026-11-20-8 and OD-2026-11-20-9 are not in this branch's `docs/owner-decisions.md`.

**Scope and layers.** No change: all of it is SQL and adapter, with no domain or `serde_json` impact. I did not check other branches or worktrees again.
