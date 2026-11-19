# Architecture review of the 2026-11-19 revision of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 4c027adffaa889109a4b776c148d0f11263f9130
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Counts: Blocker 1, Major 9, Nit 12
Note: a fresh `architecture-reviewer` given the diff 90c3e17a..4c027adf and the files, no conclusions; recorded condensed by the author, one line per finding, with the answer of resolution.md.

## Blocker

- A-B1. CA took the fences and read the policy values in one statement; under READ COMMITTED its snapshot predates the lock, so a policy change committed during the wait is not seen (turning capture off included).

## Major

- A-M1. ADR-0095 called direct contradictions of accepted ADRs "refinements" (ADR-0084 decision 11's order, decision 5's session clause, ADR-0027 Amendment item 1); FR-041-020 still required a 1.7 session for capture.
- A-M2. ADR-0095's rules and "Enforced by" did not match the design: "one store function" against a four-method port; three equalities against six; a purge and delete in the lock-order test that T041-36 did not run; no check for the SDK's wait.
- A-M3. The cost bound is measured only for a root CompleteStep; FailStep, the scoped path, `reject_secret_output` and the re-run have no measure.
- A-M4. The list of transactions that record a report is incomplete (`finish` has nine callers; `refuse_scoped_output` and `refuse_scoped_content`); make the carry an explicit argument.
- A-M5. The re-run is not what `recover_success_storage_quota` does; it repeats pre-transaction work, can find the lease expired, and other finalizer raises of the step-log branch are not mapped.
- A-M6. The CI selector does not run `integration_step_logs` when the worker and execution code it guards changes.
- A-M7. The spike's seam names a type that does not exist (`PgStore`), fires on every commit, and could reach workspace builds; the acceptance rule lacks a baseline-drift bound, an offered rate for the ticket gate and the unmeasured costs.
- A-M8. A carry of up to 500 records holds the run row and the usage row for a long time; nothing measures it.
- A-M9. Owner gates on OD-2026-11-19-1 are missing from tasks that build the redesign, and nothing records the acceptance of ADR-0095.

## Nit

- A-N1. "Held after T4" names a hook T041-6 does not build. A-N2. The Verification Plan's lock-order row omits the carry. A-N3. The review folder's date. A-N4. The carry's place in the idempotency fingerprint. A-N5. A mark-only re-run must plan no marker. A-N6. No SDK drop reason for the 2-second deadline. A-N7. The rollout advice contradicts capture on by default. A-N8. T041-6 can drop "Waits for" and T041-34 fold into it. A-N9. ADR-0085's Note on rule 5's check must cover the carry. A-N10. Changed signatures reach callers not listed. A-N11. Field 13 ignored between T041-7 and T041-36. A-N12. ADR-0095's number not checked across branches (it was: 0094 is the highest used).
