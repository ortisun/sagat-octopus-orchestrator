# Architecture review of design 044, round 2 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 19d226028a2a387d77c637ac27b7bc0f88cf5f8a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The first architecture
review of this design (the session's reviewer limit refused it in round 1). The reviewer read files only, and also read
the constitution 5.0.0 draft (branch `docs/constitution-5.0.0`) and the branch `docs/temporal-parity-readiness`
(Architecture decisions 0110 and 0111, design 045).

## Blocker

1. Code runs buffer signals against Architecture decision 0050 decision 4 and Architecture decision 0030 decision 17,
   which no decision of this design supersedes, while Architecture decision 0111 (another branch) supersedes them
   differently (an opt-in inbox) and says design 044 reuses it; choose one substrate.
2. A tenant's publication would record a fleet requirement, which Architecture decision 0082 and Architecture decision
   0050 decision 9 forbid; make `code-workflows` an operator activation and refuse publication until it is recorded.

## Major

3. A pipeline requesting `--all-open` at every deploy is a move policy in all but name; a moved run references a start
   and a current version, against "exactly one version" (principle IV, `docs/architecture/overview.md` invariant 1,
   `docs/engineering/architecture-principles.md`); define the start version as an identity; Gap 044-010 is stale.
4. No task carries an `Owner-gate:` line; the owner decisions are not on this branch.
5. The command chain needs SHA-256, an HMAC and canonical JSON, which the domain may not depend on
   (`scripts/check-architecture.py`); a port in the domain, the implementation in the application.
6. The rule of which events are visible sits in an adapter; an exhaustive `RunEvent::is_visible()` in the domain.
7. Nothing checks that every waking path sets the pending flag; derive it in one place, a reconciler invariant, a test
   over every visible kind.
8. `~workflow` breaks `NodeId::parse_stored`, `is_derived` and `forward`; lease-expiry handling lives in several
   places; one predicate `is_workflow_task_step` and a parity test.
9. A terminal command can drop events of its own window not yet applied; the SDK holds it until the window is applied;
   `applied_through_seq`; a conformance scenario.
10. The claim changes for every step (sticky, pause), against "unchanged"; keep the SQL unchanged or budget the
    predicate.
11. The statement budget leaves out reads; the batched path has no equivalence check with `persist`.
12. Code sagas depart from Architecture decision 0010 decision 3 (plan from the published definition) and from
    principle III (independent compensations remain eligible); decide the table now.
13. The asynchronous update lifecycle departs from Architecture decision 0031 decisions 8 and 9 without a decision.
14. Reset of a code run breaks Architecture decision 0050 (decision 5 eligibility, decision 8 rules per event kind,
    decision 2 one transaction, the prefix ceiling).
15. A literal input has no schema, so a classified value copied into it is exported in clear; the developer-role grant
    is a security decision.
16. The closing window is smaller than the outstanding work; exhaustion undefined; the replay budget is not on the true
    maximum.
17. Query and replay-check tasks have no durable claim path without the broker.
18. Technical Plan 14 omits state the design relies on.
19. The down migration loses evidence; the check constraint locks the table.
20. No ceiling on the rate of workflow tasks.
21. `workflow:<type>` reuses the activity-name space, where `workflow:order` is already a valid binding.
22. Design 045 (activity timeouts, protocol 1.9) is missing from the interactions.
23. The worker sets `irreversible`, `highRisk`, `compensateWith` and `reconcileWith`, which are risk metadata the
    publisher must own.

## Nit

24. Register hygiene: Gap 044-005 reused; Decisions 044-040 and 044-026 not marked superseded.
25. The keyed-chain claim of Technical Plan 6.2 is false; a sticky worker's kept chain hides a corrupted cache.
26. The digest misses patch versions, ICU data and Python extension binaries.
27. The SDK metrics declare no labels.
28. The publisher's right to a secret is re-checked at resolution today, not only at publication.
29. Constitution Check row I does not say how the audited no-op holds in code; the feature 031 row omits validation.
30. The ADRs carry no benchmark data and do not say no person reviewed them; the 5.0.0 draft requires a "not offered"
    path that Success criterion 044-001 lacks.

Counts: Blocker 2, Major 23, Nit 7
