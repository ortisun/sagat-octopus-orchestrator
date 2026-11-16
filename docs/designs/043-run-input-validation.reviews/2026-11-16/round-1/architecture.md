# Architecture review of design 043, round 1 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Reviewer: architecture (agent blob c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Read files only.

Layering clean: the domain holds opaque bytes and a digest; the port and the gate in `application`, the adapter in
`infra-schema`; `infra-ir -> infra-schema` allowed; no empty mapper; the migrations only add and their downs refuse.

## Major

1. The build depends on ADR-0092, still `Proposed`, written by an agent, and no task waits for the owner's answer;
   decision 3 narrows publication (principle IV) and ADR-0092 does not state compatibility with runs in flight.
2. Row 13 parses its document in `enforce` whatever the process mode, so its `SchemaUnusable` column cannot be reached
   and `report` does not apply to it; no test.
3. The subset narrows what features 016 and 035 admit, and no owner decision approves that dependency (O043-005 open).

## Nit

4. `errors.md` still describes graded disclosure for repeats.
5. Stale texts: T043-3 tokio, T043-5 graded disclosure, T043-24 and `definitions.rs` owner role, `run_input.rs:9`, the
   checker's docstring and message, ADR-0090's "one migration".
6. The wiring rule matches only `::new(` under `apps/*/src`; ADR-0092 omits the constructor-argument option.
7. A transient validator wait is a `StoreError::Unavailable` told apart by comparing strings.
8. `deadline` hides a schema that keeps failing (busy, backstop, interrupted together).
9. Nothing enforces "never inside a transaction"; say "review only".
10. `architecture-principles.md` has no row for the rules of FR-043-016.
11. `Scheduler::with_before_child_check` ships in production code.
12. "Non-manual trigger" defined twice (inventory vs publication).
13. Domain messages repeat adapter limits as literals.
14. The `details.rule` enum will grow and the contract does not say so.
15. Register ranges and follow-up tasks out of date.

Counts: Blocker 0, Major 3, Nit 12
