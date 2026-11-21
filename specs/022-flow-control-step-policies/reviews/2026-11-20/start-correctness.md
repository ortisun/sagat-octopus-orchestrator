# Correctness review of T022-022 (2026-11-20, third set: the gate of a run's start)

Reviewer: `correctness-reviewer` subagent, read-only, given the working-tree diff of T022-022 with the spec, the plan
(T10-T14, T26-T29, Run creators), the data model, the journal and error contracts and ADR-0050/0056/0089/0094/0096 (not the
author's conclusions). The core admission path (acquire or queue, grant, timeout, cancel, replay) was found sound.

## Findings and resolutions

- B1: a run whose last node is a root `forEach` or loop finishes without giving its permit back. **Fixed** as security 1
  (admission refusal until G1197), with its test seen failing first.
- M1: lock-order cycle: an admission takes the run's active-run counter row (slot 6) then the semaphore; a commit that
  ends a gated run took the semaphore then the counter row. **Fixed:** `persist` locks the run's counter row first through
  the `SECURITY DEFINER` `run_active_counts_lock_slot` (migration `20270203000000`, D022-050 item 8). Proved by
  `run_admission::a_run_that_ends_locks_its_active_run_counter_before_its_permits_key` (held counter row, `NOWAIT` probe of
  the semaphore), red with the pre-lock removed.
- M2: Signal-with-Start of a start that queues is refused `SIGNAL_NOT_ACCEPTED_AT_START`. **Recorded, delegated:** the
  chosen design holds the signal with the queued run until its grant (D022-050 item 6, G022-010, T022-064).
- M3: schedule firing did not defer a flow-control refusal. **Fixed:** `AppError::FlowControl(_)` joins the arms that keep
  the time owed (the firing and the backfill). Its integration case is T022-064's (the schedule harness and the gated
  world are separate fixtures); not proved by a test in this change.
- M4: the evidence lacks the event, cron and other creator cases and the run-level races. **Recorded:** T022-022 stays
  unticked; the creators are T022-064, the races T022-028 and T022-029.
- m1: the counter check of `verify_run` could not fail as claimed. **Fixed:**
  `run_admission::verify_run_fails_on_a_seeded_counter_or_deadline_disagreement` (a counter off by one, a shifted deadline).
- m2: spec drift from FR-022-059's placement. **Resolved:** D022-049 supersedes D022-031's placement (delegated by
  OD-2026-11-20-6), with the static rule `run_start_errors`; FR-022-059's text is revised with G022-007.
- m3: admission counts every waiter, the grant routine only eligible ones. **Kept, delegated:** D022-050 item 4.
- m4: a paused queued run still times out. **Recorded:** G022-012 (as G022-008 for a step).
- m5: a refused child start is retried by every pass. **Recorded:** G022-009, T022-102.
- nits: the span carries only the group (by design, FR-022-033); `to_jsonb(runs.*)` is evaluated once more on a run load
  (kept for schema tolerance, as `start_delayed_until`; statement counts unchanged).
