# Responses to the correctness review of task T022-080 (2026-11-21)

The review is `rate-faults-correctness-reviewer.md` (0 Blockers, 6 Majors). Every Major is answered below with a test
seen failing before its fix, or, where the test can only follow code that was already right, a hand mutation the test
catches. The decisions are D022-073 (strict admission order, the coordinator's decision 1), D022-074 (a lowered
concurrency ceiling applies to queued runs, the coordinator's decision 2, gap G022-020) and D022-075 (these answers),
all delegate (OD-2026-11-20-6). The re-review of the answers is saved beside this file as `rate-faults-rereview-*.md`.

## Majors

| Finding | Answer | Evidence |
|---|---|---|
| Major 1: rounds 0 and 1 of the timeout race never reach the grant | Four orders of five rounds, each reaching its decision: the grant holds the run and waits under the window while the deadline passes; the grant finds the run held and skips it (`skipped == 1`); the grant commits first; the grant has two starts in hand and the second's timeout commits first (D022-075 item 1) | `rate_limits::a_grant_and_a_timeout_of_a_buffered_start_are_decided_by_the_first_commit`; hand mutation: the deadline check of `grant_from_queue` removed |
| Major 2: the set-back clock never decides while the moved grant is in the future | The grant moved five windows ahead, a pass at once with the clock read below the moved instant, no conditional assert; the waiter's grant no earlier than the moved instant plus a window | `rate_limits::a_database_clock_set_back_grants_nothing_until_it_catches_up`; hand mutation: `AND g.leased_at <= $12` in the count |
| Major 3: the place estimate and the run view use the filed limit | `RATE_PLACE_SQL` and `getRun`'s `limit` use the lower of the filed limit and the current ceiling | `rate_rest::a_lowered_ceiling_moves_the_estimate_and_the_limit_a_waiting_run_shows`, red before (`limit` 3 shown) |
| Major 4: the waiters ahead are counted over the deciding waiter's window | Counts per window length up to the window row's longest period; each waiter ahead compared with the count of its own window, in the grant routine and at admission | `rate_limits::a_waiter_of_a_longer_window_refused_by_its_own_limit_does_not_hold_back_a_shorter_one`, red before (`gates_granted` 0) |
| Major 5: the flood's admission order depends on the runner's speed | Decision D022-073: a new start queues while its key has an eligible waiter; the flood ages the first ten grants before the fifteenth start, which queues | `rate_limits::a_flood_of_a_thousand_starts_on_one_key_never_holds_more_than_its_limit_in_a_window`, red before (16 granted at admission, not 10); `run_admission::a_start_behind_an_eligible_waiter_queues_although_the_key_has_room_and_one_pass_grants_both_in_order` and `rate_gate::a_start_behind_an_eligible_waiter_queues_although_the_window_has_room_and_the_next_pass_grants_it`, both red before |
| Major 6: every crash comes before the decision's first write | A second crash point in T20, T21 and T24: an uncommitted journal row at the run's next sequence number holds the deciding transaction on the journal's primary key after its gate writes (checked: it holds the row-write lock of each gate table); killed there, the journal, the rows and the counters are unchanged | the three crash tests of `rate_limits`; hand mutation: the gate writes committed in a transaction of their own before the journal |

## minors and nit

| Finding | Answer |
|---|---|
| minor 1: the cancellation race does not assert the run cancelled | Asserts `CANCELLED`, the slot row each order leaves and no `rate_limit.returned` |
| minor 2: T23's "never grants early" cannot fail | `per: second`; the waiter's grant is no earlier than the recorded lease plus one second |
| minor 3: the journal records the filed limit, not the one decided with | The grant row and `rate_limit.granted` record the effective limit (`rate_limits::a_lowered_policy_ceiling_slows_later_grants_and_revokes_none`, red before: `(3, 3)`); decision D022-071 item 3 corrected; the specification's "pinned `limit`" read as the filed limit, reported to the owner |
| minor 4: two documents still name `libfaketime` | Already amended before this answer (`plan.md`, task T022-080's amendment line) |
| minor 5: the second clause of SC-022-017 has no test | New task T022-115 |
| minor 6: round 3's margin | The deadline is three seconds ahead in every round that lists before it |
| nit 1: one ceilings read per waiter tried | Kept (D022-075 item 7): read in the grant's own transaction |
