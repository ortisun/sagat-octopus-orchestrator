# Architecture review of design 041, the third approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 6dceb5482d1a5a27b22be4018ed2df83e106137b
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, given the committed design, its companions, the ADRs, constitution 4.5.0, the architecture principles, the owner decisions and its own `architecture-approval-2.md`; no shell. Recorded condensed.

Previous findings: M1, N2 to N6, N8, N9 fixed; N1 and N7 hold in part.

## Blocker

None.

## Major

1. A final batch waits up to 1 s for its tenant ticket while holding a slot of `Class::Logs`, which `Admit` takes before the handler (`crates/api-grpc/src/limits.rs:668-680`); one tenant's waiting final batches can fill the cap and shed every other tenant. Take the class slot only after the ticket, or release it while waiting; a two-tenant test in T041-7.
2. ADR-0085's accepted text (decision 6's trigger, the backfill, the read switch in Rollback, `mustNotLeak` and "the trigger tests" in Enforced by) diverges from the design, and the correction depends on O041-025 (a); ADR-0084's Enforced by names no check for decision 11. Write a Note in ADR-0085's Revision whatever the answer.

## Nit

1. The selector misses shared code the guarantees rest on (`execution.rs`, `bindings.rs`, `reveal_grants.rs`, `identity.rs`, `quotas.rs`, `redact.rs`, `domain/src/scripts.rs`, `streams.rs`, `definitions.rs`, `execution/ad_hoc.rs`).
2. T7 puts rules in the "no rule" port (`first_uncovered`, the role refusal); P3 is in the handler, not the application.
3. The marker write in the scripts adapter, and how the control plane's switch reaches `record_execution`.
4. A line's `run_id` is not tied to its stream's run.
5. Binary rollback with a step-log quota row fails all of the tenant's quota management on old binaries.
6. ADR-0084 promises a row in `architecture-principles.md` for each check; only T041-5 adds rows.
7. The script-execution read through `StepLogReadTx` duplicates `ScriptStore`'s reader and puts transactions in `api-http`.

Counts: Blocker 0, Major 2, Nit 7
