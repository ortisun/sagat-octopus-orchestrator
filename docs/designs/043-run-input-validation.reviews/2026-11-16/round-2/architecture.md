# Architecture review of design 043, round 2 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept, numbered across the
report. The reviewer read files only and ran nothing.

The layering holds: opaque bytes and digest in the domain, the port and the gate in `application`, the runtime in the
adapter, allowed edges, additive migrations, closed telemetry labels.

## Major

1. Approval depends on ADR-0092, still `Proposed`; ADR-0090's "Enforced by" claims a tokio rule the code no longer
   applies. Fix: the owner accepts ADR-0092 (O043-007) in the same act as the approval, or the approval is conditional.
2. ADR-0050 decision 2 ("a refused admission commits nothing and leaves the time owed") is refined without a record: a
   refused input commits `INPUT_INVALID` and the time is acted on. Fix: a decision in ADR-0092 and a note on ADR-0050.
3. `reported` is counted twice per start (the check before `decide` and the recheck; a `CANCEL_SOURCE` request and its
   completion), against Technical Plan 7.
4. The pre-check outside a start uses a second pool: unbounded wait, no tenant fairness, no telemetry, outside the
   cache's weight bound; row 13 compiles twice.
5. "A stricter release ships `run-input-validation-v2`" has no ADR and no check. Fix: an ADR decision and a pin test of
   the constants and the charges.
6. The down migration's guards are untested.

## Nit

7. ADR-0090 decision 9 (the child lookup) is refined by D043-164 without an ADR record.
8. The wiring rule checks the method name, not the validator passed; an unwired `ScheduleService` skips.
9. One finding has three shapes (`Unusable`, `IrError::InputSchema`, the warning), the `/inputSchema` prefix in three
   places.
10. `AppError::InvalidRunInput` carries strings; its message says "input" for an unusable schema.
11. The inventory reads every tenant's documents with no audit; say why.
12. `failureCode` exposes every stop code, a product surface decided by an agent.

Counts: Blocker 0, Major 6, Nit 6
