# Stage 9 (Implementation Readiness Test), implementation-readiness lens, of design 045, third run (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 7675358f
Reviewer: egm-review, stage 9 (the egm-implementation-readiness lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer (applied in the fourth revision).

Counts: Blocker 0, Major 7, Nit 9

Previous run: Blocker 1 and Majors 5 to 8, 10 and 12 closed; Majors 1, 3, 9, 11 and Nit 1 partly; Major 4 and Nit 7 not closed.

## Major

1. **How a stored level-0 version is read without the fields is unsaid** (`parse_stored` never receives the level). *Answer:
   accepted* (security round 3, Major 5).
2. **The publication hold cannot work as specified** (a separate port; `publish_through` and `prepare` bypass `publish`). *Answer:
   accepted*: the hold runs inside the store's publication transaction, reached from every publication path (`publish_through` and
   `prepare` included), with a typed error.
3. **The migration's schema is left open.** *Answer: accepted*: Technical Plan 2 gives each column's type, nullability and `CHECK`.
4. **The static check cannot be built as described.** *Answer: accepted* (architecture round 3, Major 7): an allowlist of the functions
   whose SQL may name the columns and a ban on interval arithmetic assigned to them elsewhere.
5. **The replaced operator functions break the registry's lock order.** *Answer: accepted*: no operator function is replaced
   (architecture round 3, Major 4).
6. **The upgrade inventory has no task.** *Answer: accepted*: Task 045-18 builds `control-plane activity-timeouts-inventory`.
7. **`retry-by-code` sits in a slice that cannot run its delay case.** *Answer: accepted*: the delay case moves to Task 045-13, and
   `failWith {code, nextRetryDelayMs}` is marked 1.9 only.

## Nit (each accepted)

1. `RetryRules` replaces the `RetryPolicy` fields of `NodeKind::Activity` and `EffectPolicy`, and `Node::retry_rules()` returns a
reference. 2. The event keys are named in `journal-events.md`; the window comes from the event, `min_protocol_minor` from the
definition (an adapter hint). 3. The closed reasons and labels are enumerated in `contracts/errors.md`; the lease commands take the
definition, `db_now` and the host features. 4. A path step is re-checked by the domain command itself. 5. The delay is a tagged part of
the fingerprint, appended only when present. 6. `pausedAtUs` is dropped. 7. The slice table names `crates/api-grpc` in slice 3, the codec
in slice 1 only, and orders Task 045-4 before Task 045-3. 8. `integration_activity_timeouts` gets its duration entry; the scale fixture is
named by Task 045-17. 9. No counterpart task in feature 022 is assumed (architecture round 3, Major 9).
