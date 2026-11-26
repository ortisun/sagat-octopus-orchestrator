# Architecture review of design 044, round 6 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: b5c23b83c9a63c8ed42f15b4cdf881ecb683dd4e
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 5's Majors 1 and 2
are closed and 3 closed as to whether it can be built; every round-5 Nit is closed except 10, partly. Layering, the code
claims checked against the tree, the arithmetic, the supersessions and compatibility hold.

## Major

1. "Terminate" does not exist (Architecture decision 0050 decision 5): the remedy is cancellation, which stops a run
   under review at once.
2. `sagat_move_code_run`'s last-row check refuses every move off a review.
3. `UNHANDLED_EVENTS` and `OUTSTANDING_OBLIGATIONS` mapped to gRPC errors although the completion commits: answer them
   as an outcome value; a contract test.
4. A binding with workflow types and no activities cannot be stored, listed or profiled with today's binding store.
5. The call-site check counts generic names: count only the distinctive ones and hold the definition loaders.
6. A reset drops `run.code_version_moved` and with it the movers from the initiators: `COPY`; the resetter too.
7. The source-text checks do not run on the pull requests that would break them: host them where scoped selection
   runs them, or call them release gates.

## Nit

8. "The 6-commit path" against Non-functional requirement 044-002's 7.
9. Section 9 against Requirements 044-019 and 044-031 for an open scope.
10. `runs_stop_reason_valid` must admit `Completed`; `code_can_act_count` missing from the all-or-null check.
11. `policies` is crate-private; `is_structured` has no answer.
12. Architecture decision 0109's protocol-negotiation test is in no task.
13. The old-reader refusal of a code document has no test.
14. The 5.0.0 draft applies a move only while the journal has not advanced past the verified activation.
15. Restored rows: a code run's projections must include them (Architecture decision 0050 decision 5).
16. Update arguments and results and the declared state have closed schemas: carriers, or say why not.
17. The chain port must be a domain trait, not an application one.
18. One application-level history projection for the lease and the export.
19. The scheduler's `leases` pass is missing from the expiry parity list.

Counts: Blocker 0, Major 7, Nit 12
