# Stage 6 (Comprehension Test) of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 67cc649e
Reviewer: egm-review, stage 6 (the egm-comprehension lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the paraphrase is kept as the reviewer wrote it; the findings are condensed by the author, every finding and its
severity kept, each with the author's answer.

Counts: Blocker 0, Major 6, Nit 18

## What this proposes

This design adds three new timeouts to `activity` and `script` nodes, in the root graph and in loop or collection bodies:
schedule-to-start, schedule-to-close and a per-node heartbeat timeout. The existing `timeoutMs` already acts as start-to-close.
Authors also choose who renews the lease: the host for as long as the handler runs (`host`), or only the handler's own
`heartbeat()` calls (`handler`). Retry policies gain a list of codes that are never retried and per-code overrides
(`byErrorCode`), and a worker can set the next retry's delay. Each deadline is stored as a column on `steps`, set from the
database clock by the transition that starts it. The claim refuses steps whose deadlines have passed, and a new
`step_deadlines` scheduler pass ends steps that are `READY` or waiting at a gate. A deadline that ends a step with no live
attempt is a known failure and takes the failure route. A deadline that ends a live attempt leaves the outcome unknown, so the
step goes to reconciliation or review instead of retrying as Temporal would. The handler-heartbeat fields and the worker's
delay ship in protocol 1.9, and a version can only publish once the operator has recorded the fleet requirement
`activity-timeouts-v1`.

## How the existing system works, per this document

A step becomes `READY` when it is released and past any gate, and a worker session claims it as a lease (`LEASED`, then
`RUNNING`). The lease has one platform-wide time-to-live of 30 s, and the host renews it as long as the handler runs.
`attempt_deadline_at` comes from `timeoutMs`, and the lease's expiry is capped at it. A lost-lease pass, run per shard under the
run lock, ends lapsed attempts with `LEASE_EXPIRED` or `ATTEMPT_TIMEOUT`. Those are outcome-unknown codes, so feature 037 sends
the step to `reconcileWith` or to review. A retries pass releases `RETRY_WAIT` steps when their wait ends. `RetryPolicy` treats
every failure code the same: the worker's `retryable = false`, a stop request, or `maxAttempts` decides. Protocol minors are
negotiated per session (1.7 is merged, 1.8 is pending). Fleet capabilities are gated by `platform_requirement_activate`.

## Findings

Major

1. The column table says the claim clears `start_deadline_at`, while a withdrawal "keeps" it. *Answer: accepted*: the claim does
   not clear it; it is replaced when a new attempt becomes `READY` and cleared when the step ends.
2. "Delivery acknowledgement", "provisional lease" and the 18 s rule are undefined. *Answer: accepted*: one sentence each and the
   branch and path of Architecture decision 0099.
3. The Risks row's gate for `heartbeatFrom: "handler"` before 1.9 exists nowhere else. *Answer: accepted*: a second requirement,
   `activity-timeouts-handler-v1`, recorded only once every engine offers 1.9, gates the handler source and the worker's delay
   (Requirement 045-017, Technical Plan 13).
4. What follows reconciliation's `not_applied`, and whether the close deadline bounds reconciliation or an operator's retry, is
   unsaid. *Answer: accepted*: reconciliation is not bounded by the close deadline (it is a derived node, Requirement 045-012);
   `not_applied` and `unknown` wait for a person as feature 037 says; an operator's retry is a person's decision and is not
   bounded by the close deadline either; its attempt gets `timeoutMs` only (Technical Plan 4).
5. `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT` are new codes although the text says "no failure code is added".
   *Answer: accepted*: the text now says two known-failure codes are added and no outcome-unknown code; Task 045-1 registers them.
6. "Fence" is undefined and the pass ends a step "with no fence" while a task says "under its fence". *Answer: accepted*: a fence
   is the pair (attempt, generation) of a lease; the two timeouts end a step with no lease fence, guarded by the generation read.

Nit (each accepted unless said)

7. `TASK_NOT_STARTED` undefined: defined (the hand-back code of protocol 1.8).
8. Generation, chain lock, shard window and owner, scoped and path statements undefined: a clause each in Terms.
9. "The deadline pass" used before its name: named at first use.
10. Row 6's `retryRule` value missing: `closeDeadline`.
11. Row 6's replay claim leaves out the jittered delay: a replay applies the journaled decision; it does not recompute it.
12. Success criterion 045-004's wording on attempts: reworded to the step's count.
13. One or two migrations: two (the columns, then the validation of the checks), said everywhere.
14. CUJ 3 needs a 1.9 host: said.
15. Success criterion 045-003's start of measurement: from the handler's last `heartbeat()` plus one beat interval.
16. The first beat moves `LEASED` to `RUNNING`: said.
17. The metric labels `heartbeat` and `lease`: explained.
18. Slice 3 depends on 1.8's server side: Task 045-9's 1.9 part is split into its own task.
19. Managed runner nodes and Gap 045-003: described.
20. Owner questions 045-007 and 045-008 missing: said where they went.
21. The retry fields also apply to `compensationRetry`: said in the Overview.
22. Unexplained references (the migration checklist, storage classification, build proofs, `heartbeatIntervalMs`): explained.
23. Architecture decisions cited by number only: linked where they are on `develop`, branch and path given where not.
24. What applies when `timeoutMs` is absent: the comparisons apply only between declared fields; `timeoutMs` stays optional.
