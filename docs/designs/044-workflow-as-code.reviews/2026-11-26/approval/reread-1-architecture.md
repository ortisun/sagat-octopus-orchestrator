# Targeted re-read of design 044 for approval: architecture, first pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: eeee9d0ee2c8c4e5074b56fc977f9838158d78a4
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since round 11 was reviewed. All three round-11 Majors are closed. Every finding below was fixed in the next commit.

## Major

1. Chain-addressed update keys have no storage key: `code_update_transitions` is still unique by run.
2. A move whose target changes the gated entries or the state's reserve passes the check and then diverges on replay.
3. Releasing a crashed lease's uploads "with its charge" in the claim contradicts Architecture decision 0027.
4. `steps.unavailable_at` is neither fenced nor cleared, so one refusal exempts every later expiry; no metric.
5. `NO_TARGET_WORKER` ends checks a worker still holds.
6. The constitution conditions cite a decision not recorded on this branch and a version not merged; no task waits.

## Nit

7. Step 2 names `lock_member_chain_sql`; run transactions use `lock_chain_for_run`.
8. The `RETAINED_BYTES_LIMIT` row still says 3 MiB.
9. `RETAINED_BYTES_LIMIT` is missing from the update reasons.
10. `REPLAY_CHECK_ABANDONED` missing from the metric and Architecture decision 0105; `COMMAND_VALUE_LIMIT` missing from the cause table.
11. Cancellation wording leftovers; the event's `CancelOrigin` not in the event contract.
12. The can-act index predicate and a writer test for `code_step`.
13. The inbox column belongs to design 054.
14. A semaphore test describes an impossible state.
15. A test prefix places integration tests in the source-text file.
16. Task 044-33 may add a route before its file exists.
17. Architecture decisions 0107 and 0108 do not name the new tests or the gated input's limit.
18. Which cause refuses a run's 65th literal key.

Counts: Blocker 0, Major 6, Nit 12
