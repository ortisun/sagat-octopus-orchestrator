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

## Second round

The short re-reads at `4d6fe4cc` (architecture, security, correctness, and the zoom-out, comprehension, design-critic
and readiness re-readers of egm-review) left the Majors below. Each is answered here, documents first; two decisions are
new, D041-134 (wait slots across tenants, a top-up that never waits) and D041-135 (how SC-041-007 is judged at 1 × 16).

- **architecture N-1, readiness R-6 (the connection hand-off):** section 2b and T041-36 state the construction: the
  report transactions hold a `PoolConnection<Postgres>` and send `BEGIN`, `COMMIT` and `ROLLBACK` themselves; a variant
  of `begin_safely` sends `BEGIN` inside its spawned task and marks the connection `close_on_drop()`; `commit` clears it
  only when `COMMIT` succeeds, then moves the connection into the top-up task when `top_up` is true; a dropped or failed
  transaction closes its connection; the helpers take `&mut PgConnection`; a fault test shows no connection returns to the
  pool inside a transaction. The budget is unchanged.
- **architecture A-6 remainder, correctness N-1, zoom-out Z-3 remainder, security N-3 (`failed` missing):** `failed` is a
  planned mark-only reason in data-model.md §1, §8a and T041-36 ("outside the four"); T041-36's re-run test is split: a
  guard's raise re-runs with a mark-only carry whose reason is `failed`, two transactions, the final mark kept; a failed
  call re-runs with none and no final mark. The re-run tier of ADR-0095 decision 5 is kept: removing it would change an
  accepted ADR's decision; that keeping is recorded for the owner's acceptance row.
- **security S-1 remainder, design critic N-3, correctness N-5 (the bound across tenants):** decision D041-134: a waiter
  holds a tenant slot (*P* = ⌈*Q* / 2⌉) and a replica slot (*Q* = max(1, ⌊N / 4⌋)), 3 and 5 at the default pool of 20, so
  step-log work holds at most 10 + 5 of 20 connections and at least 5 are never held by it (2 of 8 at a pool of 8); *Q*
  reaches SQL through `sagat.step_log_wait_slots`, set by the pool at connect; T041-36 tests the replica cap with five
  waiting tenants and a sixth answered busy, and B's heartbeat and task lease getting their connections; T041-37 counts
  only slot holders (`pg_locks`) and adds a regime with three or more tenants in the fallback regime. The claims in
  section 2b and the runbook are restated to what the slots bound.
- **correctness N-4 (the top-up near the limit):** decision D041-134: the top-up tries the usage row `NOWAIT` only, takes
  no slot, never waits, reclaims no other quota lease, and grants only when its share is above the writer's room
  (section 2b, ADR-0097 decision 3b, data-model.md §3b, T041-5).
- **correctness N-2, design critic N-1 (the 1 × 16 floor's statistics):** decision D041-135: T041-9 runs twelve rounds and
  judges on the median of all valid rounds (at least six); the false-stop rate of an unchanged build is stated, 5.49% per
  gate with six valid rounds and 2.38% with twelve (20/364 and 924/38,760); a miss goes to the owner with the record,
  never re-run; the (2b) floor is stated to admit about +94% of p99 (67.2 ms against off-1K's 34.6 ms), so T041-9 reports
  to the owner a p99 above the accepted +36.9%; the risk row no longer claims more; the eight valid rounds' gate values
  are recorded in validation-report.md.
- **correctness N-3, design critic N-2 (gate (4) at an unmeasured setting):** decision D041-135: gate (4) is judged at the
  cap and tickets of 10 the fourth run measured; T041-37 measures ticket refusals at the shipped defaults (cap 10,
  tickets 7), and 1% or more raises the default by a design change.
- **comprehension K-7 remainder:** section 2a's heading paragraph now says a held or short quota lease leads to the
  fallback refill, never to `contended`.
- **design critic D-4 remainder:** section 5's "Contract" states D041-133 (`run:logs:read` and the PII reveal grant in
  every namespace, `usageWithheld: true`); the revision note lists section 5.
- **design critic D-12:** still held by the critic; accepted as recommended (owner row).
- **readiness R-11:** `holdAnswer` is written in the harness's `faults[]` form (`channel`, `match`, `action`, `times`,
  `after`, `maxHoldMs`), applied by each SDK runner's transport hook as every fault is, with the observation `heldAnswers`
  in `out.json` and its normalisation to `under2000` or `atLeast2000`.
- **readiness R-13:** T041-5 states that the execution engine and the control plane create their instance id before
  opening their pool and pass it to `pool_config` / `open_pool` and `registry::member`, with the files; the one-shot
  processes use `sagat-octopus-orchestrator/<process kind>`; the pool also sets `sagat.step_log_wait_slots`.
- **readiness N-1, architecture Nit 1, zoom-out Nit 2 (the privilege list):** one list everywhere: six definer functions
  and two invoker helpers, the counter sequences revoked; T041-5 creates the helpers and the sequences; the Verification
  row, ADR-0097 decision 1 and data-model.md §3b and §4 agree.
- **readiness N-2:** `sagat_quota_step_log_reclaim_idle` answers `(reclaimed integer, skipped boolean)` (data-model.md
  §3b, section 2b, T041-5, T041-10).
- **readiness N-3:** T041-37's `pgbouncer.ini` sets `max_prepared_statements = 200` (sqlx's named prepared statements),
  `auth_type = scram-sha-256` and an `auth_file` generated by `run.sh`; PgBouncer 1.21 or later, the version pinned in
  `run.sh` and recorded.
- **readiness N-4:** T041-36's tests include the source-text check that only the store's top-up calls
  `sagat_quota_step_log_top_up`.

Nits fixed with these: architecture 1 to 5 (the warm pin is observed through `pg_stat_activity` and the top-up's call
count; the top-up takes no slot), 3 (`docs/adr/README.md` and ADR-0097 option 1's title) and 7 for G041-018 (its cell
begins "Closed"); security N-1 (the session-setting exception stated) and N-2 (`hashtextextended` single-bigint keys);
correctness 1 (the cap default), 2 (the held-lock tests replace the budget function) and 3; design critic n-1 ("rounded
in the build's favour"); zoom-out's Nit to Z-1 (Complexity Tracking states the fallback-only baseline as measured and
acceptable, and T041-37 gains a criterion that fallbacks stay at most 0.31%) and its Nit 1 (the slots' rejected
alternatives); comprehension's Nit on "late" (FR-041-005 says what it means); readiness Nits 5 and 6 (shard 0 from
`Keeper::shards`, the gauge cleared) and 8 (`pool.rs:11-24`).

## The reports of the second and third rounds

The second round's seven reports (at `4d6fe4cc`) are in `round-2/` and the third round's five (at `c52b5d37`) in
`round-3/`, each the reviewer's final message as returned, unedited (recovered on 2026-11-21 from the session's
records: they had not been saved before). File names: `architecture.md`, `security.md`, `correctness.md` (the
repository reviewers), `design.md` (egm-design, standing in for stages 5 to 8 in the third round; the critic in the
second), `readiness.md` (egm implementation readiness), and in the second round also `comprehension.md` and
`zoom-out.md`.

## Third round

The third round (at `c52b5d37`) counted 1 Blocker and 17 Majors: correctness 1 and 5 (`round-3/correctness.md`),
architecture 0 and 2, security 0 and 1, design critic 0 and 5, readiness 0 and 4. Under the cap of OD-2026-11-20-17 no
further full round runs: each Blocker and each Major with a known fix is fixed here, documents first, then one final
confirmation read of these fixes. The owner answered three questions (OD-2026-11-21-1 to -3); the rest are decisions
D041-136 and D041-137 under OD-2026-11-20-6.

### The Blocker

- **correctness B-1, design critic Major 1 (the 8 × 128 gate (2b) under "the median of all valid rounds"):** the owner's
  answer "Derived floor, like 1×16" (OD-2026-11-21-1): gate (2b) at 8 × 128 is at most +112.9 ms over the allowance,
  the worst of its eight valid rounds (+112.86 ms, round 6) rounded in the build's favour, a tolerance of 129.9 ms from
  the median of the rounds used (−17.0 ms); SC-041-007, T041-9, the runbook and decision D041-136 state it with its
  false-stop rate.

### The Majors

- **correctness N-2 remainder (the false-stop rate):** at least seven valid rounds (worst case at most 3.85% per gate at
  m = 8), the rate per m from 7 to 12, the union bound per release (at most 34.1% at m = 8 and 20.1% at m = 12, counting
  gates (1) and (3) at 8 × 128 by a resampling estimate, since they keep the fixed 90%), the rule for fewer than seven
  valid rounds (a new run, then the owner) and the median of an even count (the mean of the two middle values; the
  formula is then an upper bound): SC-041-007, T041-9, D041-136.
- **correctness M-1, architecture N-2, readiness M-1 and M-2 (the connection hand-off):** decision D041-137 (1): an
  `OwnedTransaction` that sends `BEGIN`, `COMMIT` and `ROLLBACK` itself, with a state `idle`, `open` or `unknown`; a
  drop while `open` or `unknown` rolls back in a spawned task and returns the connection, and only a failed or timed-out
  `ROLLBACK` closes it, so `close_on_drop()` is never cleared and no ordinary drop churns a backend; the top-up is the
  last statement on the same connection, after `COMMIT`, off the run lock (OD-2026-11-20-18's meaning kept); it serves
  every `PgRunTx` and `PgPathExecutionTx` command; T041-36 lists `primitives.rs`, `chain_progress.rs`, `breaker.rs`,
  `declared_operations.rs`, `execution/scoped_events.rs`, `execution/database.rs` and `pool.rs`, gives
  `begin_in_transaction`'s new parameter type, and its fault test in `cancellation.rs` shows the same
  `pg_backend_pid()` across commits, a contended claim and a refused report on a pool of one.
- **architecture N-3, security NF-1, design critic Major 3, readiness M-3, correctness M-4's pooler half (*Q* behind a
  transaction pooler):** decision D041-137 (2): *Q* is set per transaction, transaction-locally, in a statement the
  store already sends (the carry's `p_wait_slots`, a batch's T1, the marker's first statement), never at connect;
  section 2b, data-model.md §3b and §8a, ADR-0097 decision 4 and the runbook state the server-pool sizing rule
  (Σ (⌊N / 2⌋ + *Q*) plus one a replica: 128 for 8 replicas of 20); T041-37 sizes PgBouncer by it, checks *Q* and the
  timeouts through it (`options` handled as the pinned version allows, else `ALTER ROLE … SET`), records `SHOW POOLS`
  (`cl_waiting`, `maxwait`) and runs the three-tenant regime behind it.
- **correctness M-4's other half (the bound covers less than it said):** section 2b states the bound for a replica's own
  pool and for step-log waits and Logs calls; reports queued on a run row behind a fallback are named outside it, and
  T041-37's fan-out regime gains a victim tenant with its criterion and consequence.
- **correctness M-3 (the replica cap couples tenants through loss):** section 2b, a new risk row and ADR-0097's
  consequences state the channel; T041-37's three-tenant regime gives B 64-record carries with a 1 KiB output and
  bounds B's `service_busy` carries at 0.1%; A1 to A3 are excluded from the counted regimes as A is (D041-137,
  applying OD-2026-11-21-3).
- **correctness M-2 (the top-up's `limit` and `held`):** the top-up answers `limit` when its share is 0, tested before
  `room`, and has no `held` answer (it reclaims nothing): section 2b, data-model.md §3b and §7, ADR-0097 decisions 3b
  and 8, T041-5 and T041-36.
- **readiness M-4 (the benchmark replicas share one `application_name`):** T041-5: `replica_with_pool` names each pool
  `sagat-octopus-orchestrator/<a fresh UUID>` and `PoolConfig::new`'s default is `sagat-octopus-orchestrator/unnamed`;
  T041-37's setup asserts distinct names in `pg_stat_activity`.
- **design critic Major 2 (L4 and the Overview before D041-134):** L4 says the top-up is tried without waiting only and
  takes no slot, and that a refill waits holding a tenant and a replica slot; the Overview names both slots.

### Nits fixed with these

Correctness 1 (section 2a's top-up row reclaims nothing; data-model.md's "the top-up the refill" and the refill's
`bounded` parameter removed), 2 (the reported statistic is the median over the valid rounds of the p99 in percent), 3
(B's heartbeat check runs on a pool of 8 with the Logs class filled), 4 (`SKIP LOCKED` plus an unlocked existence
check instead of `NOWAIT`, so neither the refill nor the top-up opens a subtransaction) and 5 (below a pool of 16 one
tenant's tickets equal the cap: stated in data-model.md §6 and the runbook); correctness Nit 6 (Nits 4 to 6 of its
second report): the second report is now saved (`round-2/correctness.md`) and its Nits 4 to 6 are answered here: 4,
`path = refill` is defined as "its block ran" (its fallback refill granted, or, in a race, the block's charge found room;
data-model.md §8a); 5, T041-37's cold-start row has no warm-up, so its first carries are measured; 6, every count taken
from a carry's return is recorded only when its transaction commits (data-model.md §7). Architecture 1 (the old top-up wording), 2 (the two files), 3 (ADR-0097's header names D041-134 and D041-137
and how they are accepted) and 4 (10 in the binary, 20 in the chart; the engine's `cli.rs` help is stale). Security 1
(no reserve at a pool of 2 or less: stated, the runbook advises at least 3), 2 (A1 to A3 excluded, B judged), 3 (the
slots are taken before the insert of an absent row) and 4 (S-1's N-4: anyone who sees `ceiling = tenant_quota` already
holds the PII reveal grant in every namespace, D041-077, so D041-133's condition is met by construction). Design critic
1 (the rate per gate and per release), 2 (fewer than seven valid rounds), 3 (the owner report does not stop the
release; the runbook's and the release notes' figures are restated), 4 (the newest revision note lists the second and
third rounds), 5 (the bound is on step-log waits and Logs calls) and 6 (workload (b) from ten tickets). Readiness 2 (the
one-shot processes listed, `PoolName`, the registry pool's name, *Q* computed and never configured apart), 3 (`held`),
4 (data-model.md §3b), 5 (the top-up's helper lives in `step_logs.rs`), 6 (the SCRAM verifier read back from
`pg_authid`), 8 (fewer than seven valid rounds) and 9 (the fault test in `cancellation.rs`, which T041-36's Files list).
Readiness Nits 1 and 7 (the `releaseAnswer` and `heldAnswers` placement; the reason after a `MarkOnly(failed)` re-run)
are left to the build of T041-14 and T041-36, where the goldens and the plan type are written.

## Final read

The final confirmation read of the third round's fixes ran once, at `54341713`, by the same five reviewers; their
reports are in `final/`, verbatim (`architecture.md`, `security.md` and `correctness.md` with the header lines the
approval check reads; `design.md` and `readiness.md`, which `final/egm-review.md` summarizes). Totals: no Blocker;
Majors: architecture 1, security 0, correctness 1, stages 5 to 8 3 (D-12 among them), readiness 0; Nits 30.

Under the cap (OD-2026-11-20-17) no further round runs and no Major is fixed after this read: each is accepted by one
owner row, OD-2026-11-21-200 to OD-2026-11-21-205 (Z-2 of the first round with them). Two of them carry a residual the
coordinating session puts to the owner: the owner report's threshold on the p99 (OD-2026-11-21-200 and -205: it fires
on the accepted build's own rounds, so its threshold is derived from the eight valid rounds' figures when task T041-9
records them) and the pooler's server-pool sizing rule (OD-2026-11-21-201 and -204: heavy multiplexing behind a
transaction pooler is not bounded, and task T041-37's pooler rows are measured at the rule's size).

The Nits stay open in the approved text; the build follows the reviewers' corrections, and the task named settles
each:
- the top-up's `held` still listed at design section 2b's metric paragraph and the `NOWAIT` of data-model.md §8a
  (correctness 1 and 2, design 4): the built code has no `held` answer and tries `SKIP LOCKED` (T041-5, T041-36);
- a negative share (F < 0 after a lowered limit or with enforcement off) answers `limit`, with a test (correctness 3,
  T041-5);
- the refill's and the top-up's freedom from subtransactions is checked by a source-text test that their bodies hold no
  `EXCEPTION` (correctness 4, T041-5);
- the "no runtime" branch of `OwnedTransaction`'s `Drop` calls `detach()`, since `PoolConnection::drop` spawns
  (correctness 5, architecture 2, readiness 4, T041-36);
- the pooler rows judge the pool's `maxwait` and B's heartbeat and task-lease latency measured by the driver, not "B's
  `maxwait`"; the sizing rule counts the pooler's other clients and the configured Logs cap (correctness 6, security 1
  and 2, design 7, architecture 1, T041-37 and the runbook);
- the per-release false-stop figures are estimates where they add the resampled gates (correctness 7, design 10), and
  the owner is shown them with the approval (the coordinating session's report);
- counting a carry only when its transaction commits undercounts one whose `COMMIT` outcome is unknown but which
  committed (correctness 8, accepted: the stream row stays exact);
- `definitions.rs:164-165` and `primitives.rs`'s own helpers take `&mut PgConnection` (architecture 3, readiness 1,
  T041-36);
- ADR-0097's "Enforced by" gains the owned-connection fault test, the per-transaction *Q* tests and the pooler checks,
  and its header cites OD-2026-11-20-17 rather than rules 2 and 4 (architecture 4, design 9), with the ADR edit of
  T041-36;
- the Owner-acceptance line and T041-9's Owner-gate cannot change in an approved header and body; OD-2026-11-21-1 to -3
  cite the design (architecture 5, design 6);
- the revision note's list misses section 2's admission paragraph and tasks T041-6 and T041-7 (design 5);
- below a pool of 16 one tenant's tickets equal the Logs cap (design 8): stated in data-model.md §6 and the runbook;
- the batch's and the marker's *Q* each get a test that a transaction held after its first statement reads the store's
  *Q*, and data-model.md §8 shows the set_config (readiness 2, T041-6 and T041-13);
- one function in `crates/infra-postgres/src/pool.rs` computes *Q* from the pool's maximum, tested in T041-5 and called
  by both stores (readiness 3);
- the owned-connection test reads `idle in transaction` after a fresh acquire of the pool's only connection
  (readiness 5, T041-36);
- readiness 7 and 8 (the `holdAnswer` goldens; the reason a `MarkOnly` carries) are settled in T041-14 and T041-36;
- security 3 (the timeouts behind a pooler that drops `options`): accepted, the runbook says to set them on the role.
