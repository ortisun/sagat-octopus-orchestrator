# Responses to the re-reviews of the rate gate (2026-11-20)

Re-reviews: `rate-gate-rereview-correctness-reviewer.md`, `rate-gate-rereview-security-reviewer.md`,
`rate-gate-rereview-architecture-reviewer.md`, over commits a3a2d2d3 and a67ba790. Decisions: decision D022-066 in
`../../research.md`; the owner's answer on the rate count is OD-2026-11-20-11 (`docs/owner-decisions.md`). Each finding
was checked against the code; each fix of a Blocker or Major is shown by a test seen failing (by the code before the fix,
or by the hand mutation named).

| Finding | Resolution |
|---|---|
| Architecture Blocker 1, security Major 1 (an aged window revived without the ceiling) | Fixed (decision D022-066 item 1): `rate_limit_live_keys` counts window rows again, so an aged row keeps its place and its reuse adds no row. `rate_gate::an_aged_window_counts_against_the_live_keys_ceiling_until_it_goes`, red before the fix (the new key was admitted) |
| Correctness Major 2, architecture Major 1 (window rows unbounded) | Fixed (decision D022-066 item 1): the rows are bounded by the ceiling; a decision that lacks room reclaims its own namespace's purgeable windows of at most 64 grants (`SKIP LOCKED`, a second statement under the locks, the purge's predicate shared by `purgeable_grant!`). `rate_gate::a_new_key_at_the_ceiling_reclaims_a_window_only_purgeable_grants_keep`, red under the hand mutation that makes the reclaim return 0 |
| Correctness Major 1 (lost wake-up: `least` and the rest's old snapshot) | Fixed (decision D022-066 item 2): the rest locks the window, then computes in a second statement. `rate_gate::a_rest_that_waits_for_a_grant_in_flight_counts_that_grant` (barrier: the window held by the test, the rest seen waiting in `pg_stat_activity`), red before the fix (the hint was `NULL`) |
| Architecture Major 2, correctness minor 7, security minor 4 (gates not recorded everywhere) | Fixed (decision D022-066 item 5): runbook section 6 names tasks T022-079, T022-080, T022-081, T022-086, T022-104 and gap G022-018; gap G022-016 is a release gate; tasks T022-079, T022-081, T022-086 carry the gate. `run_creators::a_reset_of_a_rate_limited_start_is_refused_until_its_rules_are_built` pins the interim refusal (synchronous and pending), red under the hand mutation that drops the reset condition |
| Security suspicion 4 (the owner's acceptance named only the concurrency count) | Closed by the owner, OD-2026-11-20-11 ("Yes, extend it"); decision D022-066 item 6; `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed` pins the bound, red under the hand mutation that reads the live count as zero |
| Correctness minor 1, architecture minor 1, security suspicion 1 (the rest's anchor lags) | Fixed (decision D022-066 item 3): `GateKey::selected_at` |
| Correctness nit (`least` with a null writes `infinity`) | Fixed (decision D022-066 item 4) |
| Security minor 4 (the rotation of a key in continuous use; a backlog query) | Fixed in runbook sections 2 and 6 |
| Security nit 6, architecture minor 9, correctness minor 4 (the usage counts every version) | Kept, recorded: the same convention as `concurrency_live_keys`' usage; the usage now counts the rows the gate counts |
| Architecture minor 7 (the live predicate cannot use the index) | Gone with item 1: the count reads only window rows |
| Security Major 2 partly, security minor 3, minor 5, correctness minor 3, architecture Major 5 partly (purge and lease pass costs) | Open, gap G022-018 (a production gate now); tasks T022-062, T022-089 |
| Architecture Major 3, Major 4 (consistency check, barriers) | Open: tasks T022-079 and T022-081, production gates |
| Correctness Major 5, architecture Major 1 of the first review (reset refusal retryable) | Kept as recorded in D022-065 item 7, now a release gate pinned by a test (above) |
| Security minor 2, architecture minor 4 (checker rules keyed by file, not function) | Open; recorded for task T022-081's fitness pass |
| Correctness minor 2 (no test of the lease pass's lowering), minor 5 (`verify_start_rate` and G022-017), minor 6 (tests that do not fail on a reverted fix) | Open: task T022-079 (`verify_run`, `grants_total`, step scope) and task T022-080 |
| Architecture minor 2, 3, 5, 6, 8, 10, 11 (port doc, ADR-0088 revision note, principles rows, budget line of a run-level rate start, telemetry gaps, `verify_start_rate` index, down refusal) | Open; the port doc of `record_rate_leases` is corrected in this change; the rest are tasks T022-061, T022-076 (run-level line), T022-082 and T022-079 |
