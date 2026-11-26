# egm-review Stage 9 implementation-readiness lens, round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 2249e90100039f5c012b6217e4e5bb454ace1296
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange 1526-1826
Note: condensed by the author from the reviewer's output; every finding and severity is kept.

1. Blocker (1575-1586, 1599, 1619-1631): Requirement 044-034 is given to a pure domain task, but `node_for_instance` has
   128 call sites in 33 files across the application, infra-postgres, scheduler and api-http crates; no task makes the
   store load the overlay, and the signature is undecided. Decide it in Task 044-4; add a task after the migration that
   changes the load and lists every call-site file; move the acceptance to an integration target.
2. Blocker (1565-1574, 1600-1607, 1702-1712, 1780-1787): core tasks rely on Architecture decision 0111's inbox, which is
   not on this branch and has no builder in the build order; gate them on it or specify the inbox here.
3. Blocker (1580-1582, 1593, 1635, 1674-1696, 1782-1783): the golden histories, commands, digests and hashes have no task,
   path, format, reference functions or review rule; the fake server of the TypeScript lane is undefined.
4. Blocker (1534-1539, 1698-1700, 1722-1727, 1746-1750, 1767-1772, 1785-1786, 1804, 1812): the core and deferrable split
   contradicts itself (worker queries and patches in the core conformance run; the pause and the run view only in a
   deferrable task; Task 044-26 runs without Tasks 044-13 and 044-20).
5. Major (1553-1564): the contract content (proto fields, REST routes with permissions, statuses and bodies, the error
   list with statuses) is left to the writer.
6. Major (1553-1558, 1608-1618): no task changes the binding domain, service and schemas for `workflowTypes`.
7. Major (1553-1564): Task 044-2 leaves the tree broken (gRPC stubs, the cross-tenant battery's `not_built` list, the
   Studio types and the reference goldens).
8. Major (1530-1531 and the red-test lines): the new targets have no directories, `[[test]]` entries or duration figures;
   `security_workflow_as_code` breaks the naming rule; most red tests name no crate or target; the budget suite is
   unnamed.
9. Major (1565-1607, 1780-1782): tasks come before the code they need (publication before the domain; with-start before
   update admission and the inbox; creators owned by later tasks).
10. Major (red-test lists of Tasks 044-5, 8, 11, 21, 24, 29): tests the requirements name as acceptance are missing (the
    closing rule, the namespace bounds, the protocol bounds, the per-cause tests, the excluded fields, the envelope
    members, the sticky rewrite, `INCONCLUSIVE_REDACTED`, the TypeScript precondition).
11. Major (1595-1599): the all-set-or-all-null check cannot hold for the window and deadline columns; `input_digest` has
    no purpose; the pause setting has no home; the down test checks only versions.
12. Major (1628-1629): the statement budget's "plus 4" does not count the overlay read, the inbox update and the chain
    comparison; the baseline constant is unnamed.
13. Major (1619-1631): the failpoint mechanism, the interleaving generator and the differential harness are undefined.
14. Major (Technical Plan 15, 7.1): no task implements or tests the metrics, spans, the unserved alert, the version list
    or the audit records.
15. Major (1632-1697): the API of each SDK and its packaging (Node 22.8 and Python 3.11 against the SDK baselines, the
    analyzer and artifact names, the CI toolchains) are undefined.
16. Major (1746-1750, 1793-1815): the conformance harness for code workflows (scenario format, new steps, induced
    mismatch, "not offered", bindings) is not specified.
17. Major (1824-1826): the gates omit several checks and suites; names instead of commands.
18. Nit (1596-1598, 1604-1605): the privilege audit and the slot-order test name no function.
19. Nit (1641-1643): the failure-count test is in Task 044-11, Requirement 044-008 says Task 044-12.
20. Nit (1699-1700): snake_case names "in each SDK" against each language's convention.
21. Nit (1541-1548): the runbook and quickstart files are not named.
22. Nit (`docs/designs/README.md`): "0 of 28" against 29 tasks.
23. Nit (gate lines): the `Covers` of the gate rows must name the tasks; the gating is selective.
24. Nit (1743-1744): revising budgets in the approved doc makes it a draft; use a follow-up task.
25. Nit (1600-1618): the 128-character limit holds only without the `sha256:` prefix.
26. Nit (1737-1738): the runtime-role suites are not named.

Counts: Blocker 4, Major 13, Nit 9
