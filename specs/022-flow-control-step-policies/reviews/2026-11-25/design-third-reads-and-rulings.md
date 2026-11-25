# Third reads of the design of the rest of feature 022 (commit `c910345b`), and the third rulings (2026-11-25)

The three reviewers read the second revision for Blockers and Majors only. None found a Blocker. Every earlier
Blocker is closed (architecture N0; correctness B1, B2; security Blocker 1). The new Majors are refinements, and each
reviewer's verdict on every ADR is "accept" or "accept with changes". The coordinator therefore accepts ADR-0135,
ADR-0136, ADR-0137, ADR-0146 and the amendments of ADR-0050, ADR-0056 and ADR-0094 once the rulings below are written
into them, delegate (OD-2026-11-24-4); a fourth full review is not required. Decisions D022-260 upward.

## Rulings

1. *The dispatch outcome in the journal* (architecture Major 1, correctness Major 1). The `run.finished` journal row is
   appended after the two dispatch statements, its `namespace_dispatch` detail taken from the insert's outcome; it is a
   row of the run's own commit, so the leaf argument holds; the reducer ignores the field. Test: the held-commit case
   asserts each run's `run.finished.namespace_dispatch` equals its row's state, and the terminal path updates no
   `run_events` row.
2. *A later cause during draining* (correctness Major 2). Option (a): while a run drains, a further stop request
   (an operator's cancellation, the deadline, a second timeout) is refused `409` as a second stop is today; failures of
   region steps settle into the drain and add no cause; `later_causes` is removed; qualification reads the one journaled
   cause. Test: both orders of cancellation and timeout, asserting the dispatch row and that the state rebuilt from the
   journal agrees. Chosen over a journaled event per later cause, which would make qualification depend on an
   unbounded sequence for no operator benefit.
3. *The overdue pass between pick and lock* (correctness Major 3). `stop_run(TimedOut)` on a run in its handler phase
   is a typed no-op the pass counts as skipped. Barrier test: the pass held between pick and apply while the commit
   that opens the phase lands; no `failure_handler.draining` follows.
4. *Continue-as-new under a handler or test tree* (correctness Major 4). Refused at admission with typed codes: a
   dispatch fails with a typed `last_error_code`; a child admission under a handler or test tree fails its child step
   with a typed code; a draft test is refused `422`. The runtime check stays as a defence and ends the run through a
   typed finalization, as `ChainLimitFinalized` does. Test: no run stays non-terminal at the boundary.
5. *Budget deferrals* (correctness, possible Major). A deferral by the dispatch budget does not count an attempt; test:
   a dispatch deferred more than 12 times stays `PENDING` and is admitted once.
6. *The producer's locks of a hit* (architecture Major 2). An unlocked pre-read of the entry's immutable
   `producer_subject`, then that subject's membership and binding locks shared, then the entry `FOR SHARE` and the
   subject checked again (a change is `unauthorized`); at an admission the pre-read and locks are at slot 5 and the
   re-check at slot 7. Chosen over a try-lock, which would turn contention into spurious misses. ADR-0056 decision
   2(a) is rewritten on the true premise (binding replacement locks `worker_sessions` and `steps` after its key): the
   rule "no decider locks a step or `worker_sessions` row before its lookup round" is stated and checked, and
   `slot_order.rs` gains binding replacement against a lease-expiry pass that releases a cacheable successor.
7. *The `RunRecovery` role* (security Majors 1 and 2, architecture Major 3). A `RunRecovery` subject is bound to named
   namespaces (an administrator's audited setting, as worker bindings are); every route that authorizes it checks the
   binding first and answers the opaque `404` outside it; the role also holds `definition:read` (the test
   `every_role_that_may_start_a_run_may_read_the_definition_it_starts` stays as it is); it may not redrive a dispatch
   (`403`, `RECOVERY_ROLE_MAY_NOT_REDRIVE`); a migration widens `memberships_role_valid` with a `down` that refuses
   while such memberships exist; the role cannot be assigned until `failure-handler-v1` is activated; the OpenAPI role
   enum is widened in the same change. A column for the role in the plan's authorization table; a tenant-battery case
   (bound to namespace A, cannot touch namespace B); a fault test (a handler that reruns and then tries to redrive
   produces exactly one handler run).
8. *Slices that must merge alone* (architecture Major 4). The finish-path rule (T022-226) lands in slice 3, its pull
   request's first commit showing it red; the dispatch-insert rule (T022-225) in slice 10; the evidence of T022-245 and
   T022-207's handler cases in slices 11 and 12; slice 3 keeps a test that a chain finalization goes through
   `persist`'s terminal path.
9. *Orders* (architecture Major 5 and the open part of N2). ADR-0146 decision 1 reads "chain, run, the scope's rows,
   the rounds, `persist` (counter row when the commit ends the run, give-backs, the rest), usage", as T31.
10. *The draft-executions lock* (architecture, ADR-0137 change). An advisory key per namespace, held shared at slot 4a
    by admissions and exclusively by `set_policy`, which locks nothing after it but its row and audit.
11. *Leftovers* (security A, correctness M8, architecture N0). `plan.md` lines 982-985, 1031 and 1032 and
    `data-model.md` lines 642 and 651-652 are struck or marked superseded by D022-242; ADR-0136's old text (the
    run-deadline case under "Enforced by", "the revision read with the definition" in option 8 and the consequences)
    is struck; the mapping of security B and C in `design-reresponses.md` is corrected (B to D022-244, C to D022-242).
