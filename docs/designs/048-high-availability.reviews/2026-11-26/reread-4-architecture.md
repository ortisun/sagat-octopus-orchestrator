# Targeted re-read 4 of design 048: architecture (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: fa671a75725087c6eb185d325701e116c9be1ce7
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 853ac19a, not a full review. The reviewer read files only and ran nothing.

## Major

1. `... or vector(0)` keeps an unlabelled `0` beside the labelled series, so the critical alert fires on a healthy
   cluster. Fix: `or on() vector(0)`, and a rule test with two synchronous standbys firing neither alert.

## Nit

1. A `WriteTx` dropped during its commit is said to close its connection, while the spawned task keeps it.
2. The pool and the `DbHandle` take `health` separately; one constructor should build both; `Admission` collides with
   `application::events::Admission`.
3. "Only the registry's connection connects" omits a command's waiting and fresh-database connections.
4. ADR-0130's "Enforced by" omits the `test-support` rules and the alert tests; its decision 5 omits the fresh-database
   exception.

Counts: Blocker 0, Major 1, Nit 4

All five were fixed at `8464d055`.
