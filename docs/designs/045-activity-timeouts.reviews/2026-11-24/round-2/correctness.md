# Correctness review of design 045, round 2 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 5033df9d
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author; every finding and its severity kept, with the author's answer. Architecture decisions 0098 and
0099 were read from the local delivery-acknowledgement worktree. Nothing was run.

Counts: Blocker 0, Major 10, Nit 12

First round: Majors 1, 4, 6, 12, 13 and Nits 1 to 9 closed; Blocker 1 and Majors 2, 7, 8, 9, 10 and 14 not fully closed (below);
Majors 3, 5 and 11 closed with a new problem each (below).

## Major

1. **The close floor depends on the node, but no column holds it**, so the pass, the hint and the parity test cannot match the
   domain. *Answer: accepted*: the domain computes the close cutoff (the close deadline less the floor) at the release and journals
   it; `steps.close_cutoff_at` is the indexed column, used by the hint, the pass, the lease decision, row 6 and a gate's grant.
2. **Publication accepts steps that can never be leased** (a close budget at or below the floor). *Answer: accepted*:
   `scheduleToCloseMs` must be at least the floor plus 1,000 ms; a boundary test.
3. **Pause cannot be built as written, and its shift is wrong** for a step that became claimable during the pause, across two pauses
   and for path instances. *Answer: accepted, with a simpler rule* (Decision 045-019 revised): while a run is paused no claimable step
   has a start deadline (the pause clears it; a step that becomes claimable during the pause gets none); the resume gives every
   claimable step of the run, root and path, a fresh full window from the resume's `db_now`, in one statement, from a column that
   holds the node's window (`steps.start_window_us`). The close deadline is enforced in every path during a pause, the
   `step_deadlines` pass included, because it is the author's business window.
4. **The paused exclusion cites a precedent that does not exist**, and a paused flood would make every tick walk it. *Answer:
   accepted*: no run join is needed: a paused run's rows have no start deadline, and its close cutoffs are acted on.
5. **The replay floor cannot be probed** (the new transitions are an existing event type), the handler requirement protects no
   journal transition, and the process kinds are unsaid. *Answer: accepted*: the probe is the version's level (security round 2,
   Major 1): removal is refused while any workflow version of that level exists, retired included, which makes the floor
   permanent once a version used the fields (stated as the rollback limit); both requirements are recorded for control planes and
   engines.
6. **A withdrawal that keeps the start deadline turns a lost delivery into a terminal failure.** *Answer: accepted* (Decision
   045-025): a withdrawal writes the start deadline as the later of the old one and the withdrawal's `db_now` plus the shorter of the
   node's window and 6,000 ms, on `step.lease_withdrawn`, bounded by the four withdrawals per attempt of Architecture decision 0098
   decision 7; the timeout's sentence says "no lease of the attempt started in time"; a test with a healthy worker's lease withdrawn
   just after the deadline.
7. **Where the lease decision sits in the claim loop is unspecified**, and a lease error ends the run's loop. *Answer: accepted*: the
   domain predicate filters the steps that build `ready` in `claim_root_run`, and in `claim_path`, before any audit, quota or input
   work; a refusal from a lease command is then an invariant violation.
8. **The third claim test cannot fail.** *Answer: accepted*: replaced by a barrier between the candidate query and `begin_claim`, in
   which the deadline passes; the claim skips the step and leases its sibling.
9. **The 10,000 ms derivation fails for a 1.9 host and for an older host with a fixed interval.** *Answer: accepted*: the minimum is
   18,000 ms (Decision 045-024); a 1.9 host's first interval is the shorter of a third of the timeout and a third of the time left at
   receipt, and its interval the shorter of its configured one and a third of the timeout; an older host's fixed interval must be
   below a third of the shortest heartbeat timeout it serves (`sdks/SPEC.md` §6.4 says so); a conformance case with receipt at 6 s.
10. **The overlay with a derived maximum delay is unspecified.** *Answer: accepted*: an entry without `maxDelayMs` gets the larger of
    the base's maximum and the entry's initial delay; a shared fixture for the six SDKs.

## Nit (each accepted)

1. The clearing events: `reconciliation.requested` and `step.needs_review`.
2. The attempt deadline from the event for a level-above-0 version, the close deadline when there is no `timeoutMs`; the replay
   comparison excludes the lease expiry.
3. The tie: the start deadline against the close cutoff.
4. For a gated node only the grant writes the start deadline; the start deadline is cleared when an attempt ends with a report.
5. The reset rule does nothing (a reset point has no open work): dropped; the test checks the new run's fresh deadlines.
6. Requirement texts made consistent; a lease decision's skip is counted (`step_claim_skips_total{reason}`).
7. Row 6's `SCHEDULE_TO_CLOSE_TIMEOUT` is counted in the failure's transaction.
8. Success criterion 045-002 uses CUJ 2's policy.
9. The worker's delay described the same way everywhere; row 6 records `delaySource` too.
10. The minimum `scheduleToStartMs` rises to 10,000 ms, two stream windows of 5 s, so that a lost hint does not fail a step whose
    workers are up; row 6's margin is documented.
11. The gate uses the close cutoff; the waiter's cause column is named in Task 045-10.
12. Architecture decision 0110 decisions 3 and 8 amended with the design.
