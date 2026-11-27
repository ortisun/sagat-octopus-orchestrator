# Correctness review, round 1: pull request #283 (the follow-up of pull request #260, `lease-withdrawal-v2`)

Reviewed head: `3e5750bd`. The findings as the coordinator relayed them on 2026-11-25; each disposition is delegate
(Owner decision 2026-11-24-4): fix. The decision that records them is Decision 006-100 of `../../research.md`.

| Finding | What it said | Disposition |
| --- | --- | --- |
| Major (with security) | The capability migration holds three registry tables `ACCESS EXCLUSIVE` while it builds an index over `run_events`. | Fixed in `fb2ffe4c` (see the security report). |
| Major 1 | The heartbeat's pause ignores what is left of the 25 s window: a failed beat at 12 s and a normal 8 to 12 s pause leave a healthy engine refusing capability calls from 25 s. | Fixed in `4c138b6b`: `pause_before_the_next_beat` bounds every pause by the last start that can still renew (19 s after the decision) and by a second once that has passed; the paused-runtime test `a_failed_beat_is_tried_again_in_time_to_renew_the_capability_decision` (second beat at 26.9 s without the bound, red) and the edge test of the bound. |
| Major 3 | The second gate check has no test; deleting it leaves everything green. CompleteStep and FailStep while silent are untested. | Fixed in `e1c4e877`: a call held behind its run's row while the test forgets the decision (`ServingGate::forget_capability_decision`, behind the `test-support` feature) is refused and records nothing (`a_call_whose_window_lapses_while_it_waits_hands_out_and_records_nothing`); `a_silent_engine_serves_no_capability_call_and_keeps_heartbeats_and_reports` now serves `CompleteStep` and `FailStep` while silent. |
| Nit 2 | The refused call has already committed its audit and pin. | Fixed in `e1c4e877` (security Minor 2). |
| Minor 1 | `CheckedFirst` checks at the first poll only. | Fixed in `e1c4e877` (security nit). |
| Minor 2 | `Registered::Full` renews the decision. | Fixed in `4c138b6b` (security Minor 3). |
| Minor 3 | `SAGAT_DB_STATEMENT_TIMEOUT_MS=0` leaves the registry's connection unbounded. | Fixed in `4c138b6b`: 0 is the registry's 5 s; the case is in `the_registry_has_one_connection_of_its_own` (failing without it). |
| Minor 4 | The window does not advance during a suspend. | Fixed in `4c138b6b` (security nit). |
| Minor 5 | `StopAllowance::HandBack` does not depend on the session's minor, so below 1.8 a handler's `TASK_NOT_STARTED` on a cancelled `LEASED` secret step changes outcome. | Fixed in `04e83575`: granted only when the holder's minor clamped to the build's ceiling reserves the code, at the root and in a scope; `below_1_8_a_task_not_started_passes_no_cancellation` (root and scoped, red before: the failure was recorded). |
| Minor 6 | No replay of a journal holding a `NOT_ACKNOWLEDGED` withdrawal. | Fixed in `da4d65e7`: `a_journal_that_holds_a_not_acknowledged_withdrawal_replays` (SQL fixture, `verify_run`). |
| Minor 7 | No plan test that the `NOT_ACKNOWLEDGED` probe uses its partial index. | Fixed in `da4d65e7`: `the_probe_of_not_acknowledged_withdrawals_uses_its_partial_index` (failing with an index of the wrong predicate). |
| Minor 8 | Possible starvation of an activation by share-locking beats. | Measured in `28257774`: no starvation; the test stays (see the security report). |
| Nit 1 | The rollback test should compare the bodies, security and settings of both functions before and after. | Fixed in `da4d65e7`: `the_rollback_of_the_follow_up_restores_the_two_functions_exactly`, against a database that only ran the migrations up to `20270208000200` (failing with a changed down). |
| Nit 3 | The down of the capability migration must lock `run_events` in SHARE mode before it looks for `NOT_ACKNOWLEDGED` rows. | Fixed in `fb2ffe4c`: both downs (`20270211000100`, `20270211000150`) lock it first. |
