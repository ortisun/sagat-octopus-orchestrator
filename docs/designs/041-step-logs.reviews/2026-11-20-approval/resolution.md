# Approval run of design 041's revision: answers to the first round

The first round (reports in this folder, at `ff8033cd`) found 3 Blockers, 68 Majors and 86 Nits. Each Blocker and Major is
answered below, documents first, on the approval path of OD-2026-11-20-17 ("Full review, capped"). The owner answered two
findings (OD-2026-11-20-18 and OD-2026-11-20-19); the coordinating session decided the others under OD-2026-11-20-6, recorded
as decisions D041-124 to D041-133 of `specs/041-step-logs/research.md`. "Design" is `docs/designs/041-step-logs.md`.
"Accepted" means accepted as the reviewer recommended, with one owner row citing OD-2026-11-20-17.

## Owner answers

- **OD-2026-11-20-18, "Top up after the commit"** (question O041-043, decision D041-124): no transaction runs before a
  report; the carry returns `path`, `room` and `top_up`; after a carrying report commits with `top_up` true, the store runs
  `sagat_quota_step_log_top_up(tenant)` on the same connection, off the run lock, without delaying the answer; the store
  keeps no room and no backend identity. ADR-0097 decision 3b amended; ADR-0095 decisions 2 to 5 restated, decision 4
  holds again with a Note.
- **OD-2026-11-20-19, "Hide it without the grant"** (question O041-044, decision D041-133): the step-log usage needs
  `run:logs:read` and the PII reveal grant in every namespace of the tenant; G041-018 closed.

## Blockers

- **A-1** (architecture): answered by OD-2026-11-20-18 (above): design section 2b "The top-up after the report", section
  2a's table and pins (warm, warm and low, cold), ADR-0097 decision 3b, data-model.md §3b and §8a; task T041-36 tests that a
  warm carrying report runs no statement outside its budget and that the top-up runs on the same backend.
- **D-1** (critic): SC-041-007 per size (decision D041-126): the original gates at 8 × 128; at 1 × 16 a floor at the
  fourth run's figures minus a tolerance derived from its eight valid rounds (80.4%, +29.1 ms, 83.5%), fixed before any
  later round; the workloads, gates and allowance defined in the criterion; the Goals, Open Questions, risk rows,
  NFR-041-PERF and T041-9 cite it.
- **R-1** (readiness): every test that needs a carry is in T041-36 (decision D041-131); T041-5 and T041-6 keep what a plain
  insert or an ingest exercises; the Verification Plan follows; a "Tests first" rule says no task tests a carry before
  T041-36.

## Majors

### Stage 5, zoom-out (`zoom_out_report.md`)

- **Z-1:** answered by OD-2026-11-20-18: the ensure is dropped; the bounded fallback stays and the top-up after the commit
  moves steady refills out of reports with no state in the store; Complexity Tracking's rejection text corrected.
- **Z-2:** accepted as recommended (the leases are kept, OD-2026-11-20-15); the Complexity Tracking row now names the
  rejected simpler alternative (one unlocked ceiling with an overshoot proved to be at most the pool times a 24 KiB carry,
  ADR-0097 option 4), its trade, the three gaps, and that the leases leave the content class's ceiling unchanged
  (G041-024).
- **Z-3:** the contradicting text is fixed: SC-041-001's refusal bullet, section 2a's table and plan paragraph (a guard's
  re-run records `failed`, not `tenant_quota`), the re-run bound, the risk row and the Verification Plan row all say that
  quota refusals and a busy row are answered inside the call. The re-run tier itself (`StepLogRefused`, a mark-only re-run
  after a guard's raise) stays as ADR-0095 decision 5 states it: removing it would change an accepted ADR's decision, which
  the delegation does not cover (decision D041-128); reported to the coordinating session.
- **Z-4:** recorded (decision D041-132): Complexity Tracking gains the row of the plan written twice, its guard (the
  cross-language property test, D041-100) and the rejected single write path, with the reason (the ingest's verdict rules
  stay in the domain).
- **Z-5:** SC-041-007 per size (D041-126); the Goals, the settled Open Question and the risk row at "Log batches make
  completions wait" restated; "six for a long step" and "five for a short step" gone.
- **Z-6:** T041-31 withdrawn; T041-35 cut to its outcome, its four runs and decisions and links to the validation report
  and research.md, with OD-2026-11-20-13 standing in for its pass; T041-35 is ticked in the approval commit (rule 9 of
  `check-design-docs.py`); the nested-statement counts are a list in section 2a. The four revision notes stay in the
  Overview, as D-12 (accepted) records: the amendment rule asks re-reviewers to scope by them.

### Stage 6, comprehension (`comprehension_report.md`)

- **K-1:** Terms define *task lease*, *quota lease*, *lease reclaim* and *quota reclaim*; the requirements, sections 2 to 2b,
  the risk rows and the tasks (R-20) use them.
- **K-2:** Terms gain *writer*, *room*, *refill*, *fallback refill*, *reservation*, *top-up*, *b*, *wait slot* and
  *step-log lock*; the Overview defines quota lease, writer, fallback refill and top-up where it first uses them.
- **K-3:** SC-041-007 states the criterion per size (D041-126); T041-35 and T041-9 cite it; the admission defaults are
  set (D041-127) and T041-7 builds them.
- **K-4:** SC-041-001's bullet and the Verification Plan row say a quota refusal is answered in the same call; only a
  failed call re-runs.
- **K-5:** the Goals and the risk row "Each logging step adds transactions" restated for one call and no transaction
  before a report (D041-124).
- **K-6:** the Verification Plan row says the fallback waits at most *b* (with a wait slot) and then answers
  `service_busy`.
- **K-7:** section 2a scopes `contended` to CA and CC′; the charge paragraph says its cases never answer it.
- **K-8:** the third measure is a list: no server setting changed, the assertion, the derivation in the test's comment seen
  failing once; the prototype's 26 is labelled as the prototype's and the built figure is derived again.
- **K-9:** the diagram shows the stream lock the ingest waits for, the fallback refill's bounded wait with a wait slot, the
  mark-only answers and the top-up after the commit; its border fixed.
- **K-10:** "K − 1 more ingests" became "three more ingests, each on a backend of its own".

### Stage 7, critic (`critic_report.md`)

- **D-2:** defaults from the fourth run (D041-127): tenant tickets 7 (6.44 at 1 × 16, 4.22 at 8 × 128, by gate (4)'s
  formula), cap half the pool (10 of 20), headroom stated; data-model.md §6 and sections 1 and 2 updated, "placeholders"
  removed.
- **D-3:** a risk row weighs the default-on cost at small deployments, with the levers, and records that OD-2026-11-20-13
  accepted the cost of capture as it ships, on by default.
- **D-4:** answered by OD-2026-11-20-19; "Open" now says no question is open; O041-026 moved to Answered; the risk rows and
  principle V updated.
- **D-5:** moot by OD-2026-11-20-18: no transaction runs before a report; the fallback runs after the fence checks, so it
  delays the commit and never makes the report late (ADR-0095 decision 4's Note; FR-041-005).
- **D-6:** "rare" replaced by the measured figures (0.31% and 2.0% of carries refilled, the longest refill-written carry
  726.7 ms, 3.6 × *b*) in section 2, section 2b and the risk row; T041-37 has a criterion on the longest run-row hold.
- **D-7:** the Goals and the risk row restated from section 2a's table.
- **D-8:** the revision note of the owner's answers now lists its section 2b citations; a new revision note lists this
  round's changes.
- **D-9:** the third measure rewritten as a list (K-8).
- **D-10:** Terms and the Overview (K-2).
- **D-11:** "What a report can wait for" is a numbered list (when, which lock, the run lock, the bound); FR-041-005 and the
  Overview point to it.
- **D-12:** accepted as recommended (the revision notes stay where the amendment rule puts them); the stale history
  sentences it named are fixed (sections 2b and 3).

### Stage 8, intermediate zoom-out (`intermediate_zoom_out_report.md`)

- **I-1:** the Verification Plan gains rows for quota exactness and interleavings, the held-room bound, the deadlock and
  quota-reclaim cases, the top-up, the wait slots and the busy loss; the gap list runs from G041-001 to G041-027 with each
  status; principle VIII cites it.
- **I-2:** new task T041-37 right after T041-36 and T041-8, before T041-14, with each regime's criterion and consequence
  fixed before its first round (D041-131).
- **I-3:** answered by OD-2026-11-20-18: the store keeps no room, so nothing is forgotten after a batch and no ensure runs
  before a report; the marker clause is gone; T041-37 records the top-ups per carry.
- **I-4:** recorded (D041-132): the SDK rule is the rejected simpler alternative in Complexity Tracking, with the reason.
- **I-5:** decision D041-129: FR-041-006 permits the busy loss and the held refusal; the stream reports the busy loss in
  `notStoredReason`, not in `ceiling`; FR-041-011, FR-041-017, the contract, the planned OpenAPI and Studio text follow.
- **I-6:** T041-37 names the regimes that count for OD-2026-11-20-14's 0.1% rule (all but the two that hold the usage row
  past *b* on purpose), and the busy rate is recorded in every regime; the owner decisions list states the reading, which
  is reported to the owner.
- **I-7:** decision D041-130: session pooling is not required; `docs/runbooks/scaling.md` stands; T041-25 no longer amends
  it; the step-log runbook states the trade.
- **I-8:** the Owner-gate lines of T041-3, T041-9, T041-25, T041-36 (and the new T041-37) and the header gain the missing
  2026-11-20 rows.

### Stage 9, readiness (`implementation_readiness_report.md`)

- **R-2:** the purge pass's idle quota reclaim moved to T041-10 (`MaintenanceStore::reclaim_idle_step_log_leases`, its
  bound, metrics, files and the two prototype tests ported); T041-5's test says "a `DELETE`".
- **R-3:** T041-5 keeps the streams' run index (D041-112).
- **R-4:** T041-36 lists its migration files, moves the upgrade oracle with its three files, adds `sagat_step_log_carry` to
  `runtime_privileges.rs`, and runs `check-upgrade-boundary.py`.
- **R-5:** T041-36 asserts the busy reason by SQL; the route assertion is T041-11's.
- **R-6:** the seam is named (the report transaction owns its connection; after a commit with `top_up` true it hands the
  connection to a task); no ensure and no room map remain (OD-2026-11-20-18).
- **R-7:** the test pause is database-side (`pause_report_after_journal`, an `AFTER INSERT` trigger on `run_events`); the
  order-guard test uses it.
- **R-8:** every metric has one producer and a task (D041-128); T041-10 publishes the SQL-side counters from shard 0.
- **R-9:** T041-36 lists every prototype file as deleted, renamed, changed or kept, `perf-spike.yml` and the judge script
  included; T041-35's Files corrected.
- **R-10:** T041-37 states the environment, rounds, validity and median rules, the PgBouncer setup and its files, and the
  multi-tenant and held-row setup.
- **R-11:** the `holdAnswer` fault and its `releaseAnswer` op are defined in T041-14 and named in the Slice D preamble.
- **R-12:** T041-6's hook is "hold after a T8 that refilled", the refill forced by a limit change that reclaims every
  quota lease.
- **R-13:** `PoolConfig` gains `application_name` (`sagat-octopus-orchestrator/<instance id>`), with its callers, a test
  and the files; T041-5 is split into migration, Rust and tests.
- **R-14:** T041-4 lists the fitness test and its target; T041-5 extends it to both checks; T041-32 selects it.
- **R-15:** the Verification Plan row restated (K-6).
- **R-16:** T041-3, T041-4 and T041-5 carry "No longer waits" lines; T041-35 says OD-2026-11-20-13 stands in for its pass.
- **R-17:** the drop of a constraint that never existed is removed; T041-4 says so once and fixes its migration comment.
- **R-18:** the mutations are named in full with the prototype tests' path, and the label closes its list.
- **R-19:** T041-6's test split into its two cases.
- **R-20:** "quota lease" and "task lease" throughout the plan (K-1).
- **R-21:** T041-1's evidence gains a line citing this folder and OD-2026-11-20-17.

### Repository reviewers

- **A-2** (architecture): answered by OD-2026-11-20-19.
- **A-3:** SC-041-007 per size (D-1).
- **A-4:** T041-5 keeps `sagat-octopus-orchestrator` as the prefix of the per-replica `application_name`, and
  `scripts/smoke-compose.sh` matches it with `LIKE`; both are in its Files.
- **A-5:** each withdrawn cost or wait line is corrected: the Goals, the Open Questions, G041-022 (one round trip, about
  2%), the Verification Plan row, SC-041-001's "short quota lease" wording, and `docs/adr/README.md` line 37.
- **A-6:** one `DropReason` enum (D041-128) is the metric label, the carry's drop keys and the planned reasons.
- **A-7:** T041-36 adds the fitness row of ADR-0095's checks to `docs/engineering/architecture-principles.md` (in its
  Files); row 31 restated, with ADR-0097's status.
- **S-1** (security): decision D041-125, the wait slots: a tenant's waits on its usage row hold at most 7 of a replica's
  pool connections; a held-lock test with a second tenant as the victim in T041-36 and the two-tenant regime of T041-37
  with its criterion fixed before it runs.
- **C-1** (correctness): SC-041-007 per size (D-1).
- **C-2:** SC-041-001's bullet rewritten (K-4).
- **C-3:** answered by OD-2026-11-20-18: no ensure answer exists; the function plans by its own allowance read and no
  answer of another transaction is an input; the property test includes each answer of the function's block.
- **C-4:** decision D041-128: the carry returns an explicit `path`; `room` is null only with `path = none`; one enum of drop
  reasons, `failed` included; one producer per metric, the SQL sequences stated to count attempts.
- **C-5:** the kept room is gone; T041-5 and T041-36 test the top-up's answers, the boundary at 65,535 and 65,536 bytes, the
  same-backend top-up and the next carry's fast path.
- **C-6:** answered by OD-2026-11-20-18: ADR-0095 decision 4 holds again, with its Note naming the fallback's bounded wait
  after the fence checks; FR-041-005 and section 2a say so; G041-021 narrowed.

## Nits fixed with the above

Critic 1, 2, 3, 4, 5, 6, 8, 9, 10 (moot), 12, 13, 15, 16, 17, 18, 19, 20 and 21; comprehension K-12, K-13, K-14, K-15,
K-16, K-18 and K-19 (the trigger function is named `sagat_quota_step_log_insert_trigger`); architecture A-8, A-9, A-10
(moot: the top-up takes no amount), A-11 (moot), A-12, A-13, A-14 and A-15; security S-2 (moot: the top-up runs only after
a committed report), S-3 (moot), S-4 (the function builds its keys and checks its reason) and S-5; correctness C-7, C-8,
C-9, C-10, C-11 (moot), C-12, C-13, C-14 and C-15; zoom-out's two counters for one event and the broken sentence; readiness
Nits as listed in the plan's answers (1, 2, 4, 6, 11, 12, 13, 19, 21 to 26 and 29); intermediate N-1 (one busy string, the
runbook modified, the release note conditional) and N-2 (the Estimate).
