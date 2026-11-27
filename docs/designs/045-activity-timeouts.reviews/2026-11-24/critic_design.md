# Stage 7 (Critic Review), design lens, of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 67cc649e
Reviewer: egm-review, stage 7 (the egm-design lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer. The reviewer read the working tree,
which did not differ from 67cc649e in this file.

Counts: Blocker 2, Major 8, Nit 9

Confirmed against the code: the seven outcome-unknown codes, the 30 s `lease_ttl`, the 38/2 and 50/1 budgets, `sdks/SPEC.md` §6.4
and the §9 register, the field numbers, AGENTS.md rules 8, 9 and 18. Every section the standard asks for is present.

## Blocker

1. **The close-deadline index and pass pick up steps waiting for reconciliation, review or an operator hold** (constitution III:
   a deadline must not replace a pending review). *Answer: accepted*, as for the correctness review's Blocker 1: the close deadline
   is cleared on those transitions and the index covers `READY` rows only.
2. **The new claim conditions are in the wrong layer**, so a backlog of handler-sourced rows starves older hosts, and filters after
   the index read walk every ineligible row. *Answer: accepted*, as for the architecture review's Blocker 1: the conditions are in
   the candidate query's branches and the domain's lease decision, with a parity test and a budget line for unclaimable rows; the
   required `scheduleToStartMs` bounds how long a handler-sourced row can wait.

## Major

1. **The claim clears the start deadline a withdrawal must keep.** *Answer: accepted* (correctness, Major 1).
2. **Claims and retries just before the close deadline create attempts certain to end outcome-unknown.** *Answer: accepted*: the
   close floor (correctness, Major 3).
3. **Non-functional requirement 045-002 cannot be met with the scheduler's defaults** (200 per pass and tick, 500 ms). *Answer:
   accepted*: the bound is derived (at most 5 ticks for 1,000 deadlines, 2,500 ms of pauses) and the criterion is 5 s for every step,
   3 s at the 99th percentile; the pass has its own window, which closes Gap 045-001.
4. **The fleet gate of `heartbeatFrom: "handler"` is undefined.** *Answer: accepted*: `activity-timeouts-handler-v1`.
5. **Workers can send the platform's own codes.** *Answer: accepted*: Requirement 045-019.
6. **Pause, operator hold and operator retry are unaddressed.** *Answer: accepted*: Decisions 045-018 and 045-019, and the clearing
   rule of Requirement 045-004.
7. **The evidence for the 3,000 ms minimum measures something else.** *Answer: accepted*: the minimum is 10,000 ms, derived from the
   specified receive and heartbeat bounds (6,000 + 2,000 = 8,000 ms), and the measurement is no longer cited for it.
8. **Principle I's audited no-op is not addressed.** *Answer: accepted in the Constitution Check*: that clause is about a timeout
   racing a signal, which this design does not change; a late report loses as today and a pass's lost race is counted.

## Nit (each accepted)

1. CUJ 2 and jitter: `jitter: false`.
2. The option number of ADR-0110: corrected.
3. Row 6's `retryRule`: `closeDeadline`.
4. An invalid merged policy: `RETRY_OVERRIDE_INVALID`.
5. The applied migration, the down rule, the row-shape predicates: the new migration has its own check; "a non-default value"; the
   predicates include the new columns (Task 045-7).
6. The cap is on renewals only; which deadline wins when both pass: said (the attempt timeout).
7. A granted `READY` step returns its slot and permit: said.
8. The margin of the handler source: a handler calls `heartbeat()` at least every two thirds of the timeout; success criterion 045-003
   counts from the last call.
9. Claims that rest on unmerged decisions, and the skipped question numbers: their status and branch are given; the questions are
   accounted for.
