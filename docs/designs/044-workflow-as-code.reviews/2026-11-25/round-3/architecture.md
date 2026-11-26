# Architecture review of design 044, round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 2249e90100039f5c012b6217e4e5bb454ace1296
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer judged round
2's findings 2, 6, 9, 19, 20, 21, 23 and Nits 25 to 29 closed and the others partly closed (raised again below). It read
the constitution 5.0.0 draft and the branch `docs/temporal-parity-readiness`.

## Blocker

1. A prefix envelope entry may own secrets, against the 5.0.0 draft ("an envelope entry that releases a secret names its
   activities exactly"); overlapping entries have no precedence (the example's `inventory.release` would compensate
   itself).

## Major

2. A moved run references two versions while the 5.0.0 draft keeps "every run references exactly one version"; the
   owner should amend that sentence before 5.0.0 merges.
3. `signals` has a different shape here than in Architecture decision 0111; its supersession covers only versions that
   declare a buffered signal; its fleet requirement `signal-inbox-v1` is not recorded or checked.
4. Task 044-21 builds on an inbox with no gate; a delivered signal with no handler is lost at continue-as-new or the end.
5. Nothing refuses removing `code-workflows` while a code version exists.
6. Twelve `INSERT INTO run_events` statements in seven files, so "one place" is not true; the reconciler compares the
   same column it would need to repair.
7. Lease expiry is also decided in SQL; a parity test like `outcome_unknown_parity.rs` is needed.
8. The node-lookup audit is one-time and its test cannot reach the call sites; a source-text check like
   `managed_kind_sites.rs`.
9. The statement arithmetic omits the overlay read and the inbox update; code-run activity steps gain unbudgeted
   statements; the overlay read can be 8 MiB.
10. Saga dependencies miss a step issued in the same completion as the activation that delivered the outcome.
11. Worker queries answer from uncommitted memory, against Architecture decision 0031 decision 7.
12. A `202` from `updateRunState` breaks older clients; opt in.
13. A reset of a moved run: its start version and pin, and its overlay rows.
14. The history headroom is undefined; the 56,320 maximum is not proven.
15. A move item waits for ever when no target worker polls.
16. `code_query_requests` has no place for arguments and answers; the 256 waits are per process.
17. Design 045 adds `steps.min_protocol_minor`; minors are cumulative, so a workflow worker depends on features 041, 006
    and 045.
18. "Fits the target's envelope" and the publisher re-checked after a move are undefined.
19. The workflow-task step's `NodeKind` is unnamed; the claim's activity-only steps (quota, breaker, pool, context).
20. The move function does a state transition in PL/pgSQL outside the domain.
21. The history export format has no version or baseline, against the 5.0.0 draft's wire-contract rule.
22. Command inputs, markers, update arguments and declared state are charged to no quota.
23. `version: active` departs from Architecture decision 0030 decision 16 without a decision; continue-as-new inside a
    saga.
24. Projections equal a replay is tested only for the visible count; the sticky rewrite is not journaled.
25. Architecture decisions 0105, 0107 and 0108 lack compatibility, operational cost and benchmark sections.
26. Tasks that implement inside other features have no `Owner-gate:` line; the owner decisions are not on this branch.

## Nit

27. Row II says the code profiles are pure domain code; the parse is behind the codec port.
28. "Expected 1.9" remains in the register.
29. "No policy MUST move runs" should be "MUST NOT".
30. Metric label values not listed.
31. How a pause reaches registered sessions.
32. The error after three unhandled refusals is unnamed.
33. The version diff of two code versions.
34. The latency target and the 100 ms start interval.
35. The 5.0.0 draft's "within one version, explicit version markers" wording.

Counts: Blocker 1, Major 25, Nit 9
