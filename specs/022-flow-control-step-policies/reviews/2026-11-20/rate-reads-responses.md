# Responses to the reviews of task T022-078 (the rate gate's reads, 2026-11-20)

Reviews: `rate-reads-security-reviewer.md`, `rate-reads-correctness-reviewer.md`, `rate-reads-architecture-reviewer.md`.
Decision D022-067 in `../../research.md`. Task T022-078 is not ticked while a Major below is open. The re-review of the fixes is answered in the second table.

| Finding | Resolution |
|---|---|
| Security Major 1, architecture Major 4, correctness minor 3 (the estimate links runs for a caller without the reveal grant) | Fixed (decision D022-067 item 8): the estimate is withheld from such a caller; the position stays and is recorded under gap G022-014. `rate_rest::a_run_shows_its_rate_slot_and_a_waiting_run_its_place` reads as a member without the grant, red under the hand mutation that drops the check |
| Security nit 1, nit 2 (the charged-route inventory, the API reference) | Fixed: `docs/runbooks/api-rate-admission.md` row; `docs/reference/api.md` regenerated after reading its diff (one operation added) |
| Correctness Major 1 (the k-th lease for k of 2 and more untested), Major 2 (the filters on the waiters ahead untested), Major 3 and architecture Major 3 (no check of `oldest_wait_at`'s value) | Fixed (decision D022-068 items 1 and 5): `rate_rest::each_waiter_is_estimated_from_its_own_counted_lease` (red under `OFFSET 0` and under `ORDER BY ... DESC`); `rate_rest::the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate` (red with the deadline filter dropped, with the status filter dropped, and with the reading run's own pause read as false; the pass's `NOT r.paused` alone survives, since a pause stores the run `PAUSED` and the status filter already leaves it out, D022-068 item 7); the `key_consistency` row "a window's oldest wait differs from its waiters'" with `consistency::a_windows_oldest_wait_that_is_not_its_waiters_is_found` and `consistency::one_statement_that_adds_and_removes_waiters_of_a_window_leaves_its_oldest_wait`, and `rate_gate::a_buffered_start_is_granted_once_the_window_of_the_slot_ahead_has_passed` now checks the remaining waiter's instant exactly (all red with the trigger's minimum replaced by the old value, see the re-review rows); unit tests `flow_control_keys::tests::{the_estimate_is_the_kth_lease_plus_the_window_at_each_edge_of_k, no_estimate_for_a_waiter_the_pass_will_not_grant}` |
| Architecture Major 1, Major 2, security minor 2 (the page cost has no check; `nextSlotAt` scans every grant and every waiter of a key) | Fixed (decision D022-068 item 3): one index probe a period over the periods of the first 256 waiters; `rate_rest::a_page_of_rate_keys_reads_only_its_own_keys` (red with the earlier scan restored); strengthened after the re-review |
| Architecture Major 5, correctness minor 1 (no test of the estimate of a dual-gated run, nor of `waitingOn: concurrency`) | Fixed: `rate_rest::a_run_waiting_for_both_gates_has_a_place_and_no_estimate` (red with the dual-gate rule dropped), `rate_rest::a_step_waiting_for_a_permit_is_shown_waiting_on_its_concurrency` |
| Security minor 3, architecture minor 5, correctness nit 3 (the place's read has no timeout and can fail the whole read) | Fixed (decision D022-068 item 2): `rate_rest::a_place_that_cannot_be_read_in_time_is_left_out_and_the_run_is_shown` (held locks and `pg_stat_activity`; red with the timeout removed and with the error propagated) |
| Correctness minor 2 (a paused or timed-out run still gets an estimate), minor 4, minor 5, minor 6; architecture minors 1 to 4, 6 to 8; the nits | Fixed: minor 2 by decision D022-068 item 1 (`rate_rest::an_estimate_after_the_queue_deadline_is_withheld`, red with the deadline rule dropped); minor 4 by `rate_rest::one_groups_rate_keys_page_through_the_keys_nobody_waits_for` and the STEP window's case in `rate_rest::a_step_waiting_for_a_slot_is_shown_waiting_on_its_rate_limit` (403 and another tenant's 404 are the tenant battery's); minor 5, the comment corrected; minor 6, the contract text (D022-068 item 4); architecture minor 1 (`check-architecture.py` names `routes/rate_limits.rs`, its fixture test red without it), minor 6 (the plan's budget paragraph), minor 7 (unit tests of the estimate and of the listings' tags), minor 8 (`conforms` against the canonical schemas); the contract nits (`since`, `additionalProperties`, the word "eligible", the terminal `GRANTED`). Recorded: architecture minors 2 and 4 as task T022-112, minor 3 under task T022-111 (decision D022-068 item 6); the redundant unleased scan and the trigger's lock strength stay as they are (two index probes; the gate already holds the window `FOR UPDATE`) |

## The re-review of the fixes (2026-11-20)

Reviews: `rate-reads-rereview-security-reviewer.md`, `rate-reads-rereview-correctness-reviewer.md`,
`rate-reads-rereview-architecture-reviewer.md`. Decisions D022-068 items 2, 3 and 7.

| Finding | Resolution |
|---|---|
| Correctness Blocker 1, architecture Blocker, security Major 1 (a filter of the waiters ahead is missing, and the diff and the tree differ) | Not a defect of the code: the diff was taken while the hand mutation "deadline filter dropped" was applied, and the tree was read during the next one ("status filter dropped"). The restored text has all three filters; each of the two mutations is red in `rate_rest::the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate`. From now on a diff for review is taken only after a mutation run has restored and rebuilt the tree |
| Correctness Major 1, architecture Major, security minor 1 (the `EXPLAIN` test reads rows returned, not rows read) | Fixed: every grant and waiter node must drop no row by a filter or a recheck, read as fractions; red with the earlier, non-indexable `nextSlotAt` predicate restored |
| Correctness Major 2 (the STEP window's null `nextSlotAt` passes without its rule) | Fixed: the window also holds a leased grant that counts, so only the unleased rule makes it null; red with that rule dropped |
| Correctness minor 1, architecture nit (the first 256 rows are not the pass's 256) | The rationale corrected in decision D022-068 item 3; the bound and the contract's text stay |
| Correctness minor 2 (no key with two windows) | Fixed: a key with an hour waiter and a minute waiter and a lease thirty seconds old shows the lease plus the minute; red with each waiter's window replaced by the row's longest |
| Correctness minor 3, architecture nit, security nit 1 (every failure swallowed; the error's text logged) | Fixed (decision D022-068 item 2, revised): only an unavailable database is left out, logged by a closed kind; a corrupt row stays an error |
| Correctness minor 4 (`t` is the transaction's start) | Fixed: `statement_timestamp()` (decision D022-068 item 7) |
| Architecture minor 1 (the compile-time check works one way) | Fixed: one text `grants_per_key_per_pass!` spliced into the SQL and parsed at build time against `GRANTS_PER_KEY_PER_PASS` |
| Architecture minor 2 (the page-cost check runs only when its suite runs) | The principles row says so; mapping the adapter's file to the suite on every pull request is the owner's policy (OD-2026-11-06-2), not changed here |
| Architecture minor 3 (the timeout test does not pin two seconds) | Fixed: the read must end within seven seconds of the reveal grants' release, below the test pool's ten-second timeout; red with the place's own timeout removed |
| Architecture minor 4 (deferrals not carried; no retired-version cursor test) | Fixed: task T022-112 and the amendment of task T022-111; `rate_rest::a_rate_cursor_of_a_forced_version_is_refused` (red with the in-force check of the route removed); this table |
| Architecture minor 5 (the adapter decides part of the estimate) | Fixed: the store returns `PlaceReading`; `RunService` derives the place (decision D022-068 item 7) |
| Security minor 2 (the place's counted grants read an `OR`) | Fixed: at run scope only `leased_at > t - per` (decision D022-068 item 7) |
| Security nit 2 (five round trips a place read) | Kept: `getRun` is not on the hot path; one statement fewer is not worth a multi-statement string |
| Architecture nits (the shared timeout's doc, the impossible `Sort` assertion, the data-model's backticks, long lines, the contract's race case for the estimate) | Fixed |
| Architecture nit (the concurrency live-keys test belongs to another task) | Committed on its own, with the references of OD-2026-11-19-4 and OD-2026-11-20-3 |
| Correctness nit 2 (the group listing's tie order checked only against itself) | Recorded: the order is the index's, which `a_page_of_rate_keys_reads_only_its_own_keys` pins |
| Correctness nit 3 (the reading's guarantee) | Fixed: documented on `PlaceReading` and checked by a `debug_assert!` |

### Hand mutations after the fixes (2026-11-20)

Each applied alone, the binary rebuilt, the named test run, the file restored and the tree rebuilt.

| Mutation | Result |
|---|---|
| `OFFSET 0`; `ORDER BY ... DESC` (the k-th lease) | red: `each_waiter_is_estimated_from_its_own_counted_lease` |
| the waiters ahead without the deadline filter; without the status filter | red: `the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate` |
| the waiters ahead without `NOT r.paused` | green: a pause stores the run `PAUSED`, which the status filter already leaves out; kept as the pass's text (decision D022-068 item 7) |
| the reading run's own pause read as false | red: the same test |
| an estimate at or after the deadline shown | red: `an_estimate_after_the_queue_deadline_is_withheld` and the skipped-waiters test |
| the earlier `nextSlotAt` scan restored; the grant filter on an expression | red: `a_page_of_rate_keys_reads_only_its_own_keys` |
| `min(leased_at + p)` instead of `min(leased_at) + p` over the same index range | green: equal value, and it reads the window's counted grants, at most the key's limit, instead of one; accepted |
| each waiter's window read as the row's longest | green before the fix (one grant counted for both windows); the case now holds a grant thirty minutes old as well, rerun with the suite |
| the unleased rule of `nextSlotAt` dropped | red: `a_step_waiting_for_a_slot_is_shown_waiting_on_its_rate_limit` |
| the place's own timeout removed | red: `a_place_that_cannot_be_read_in_time_is_left_out_and_the_run_is_shown` (before the fix by the 60-second bound, after it by the seven-second one) |
| the place's error propagated | red: the same test |
| the dual-gate rule dropped | green before the fix (the permit's default ten-minute queue withheld the estimate by the deadline rule); red after it: `a_run_waiting_for_both_gates_has_a_place_and_no_estimate` |
| the trigger's minimum replaced by the old value | red: `rate_gate::a_buffered_start_is_granted_once_the_window_of_the_slot_ahead_has_passed` and `consistency::one_statement_that_adds_and_removes_waiters_of_a_window_leaves_its_oldest_wait` |
| the consistency row for `oldest_wait_at` dropped | red: `consistency::a_windows_oldest_wait_that_is_not_its_waiters_is_found` |
| the in-force check of the rate route's cursor removed | red: `a_rate_cursor_of_a_forced_version_is_refused` |
| the group listing's `$8 = $3` dropped | green: the service refuses another group's cursor before the statement (the route's group check); the predicate is a second guard |
| the concurrency live-keys count taking a namespace lock; read as 0 | red: `ceilings::the_concurrency_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed` |
