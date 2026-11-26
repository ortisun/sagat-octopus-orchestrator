# Architecture review of design 044, round 7 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: e0a23b09f7ddbe225e80110c2cf25f3da960b730
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 6's Majors 1 to 4
and 6 are closed, 5 and 7 partly closed. Layering, the code claims, the arithmetic and the supersessions hold.

## Major

A. The definition loaders listed are incomplete: `DefinitionCache::get` has about 15 callers, and the children pass
   reads a code parent's node by a generic name and would skip a child silently; the pending reset checks the start
   version's schema; give the cache a distinctive name, count its callers, and list which compose the overlay.
B. The expiry-parity test and the raw journal-insert rule still run only at release: move the test to
   `apps/execution-engine/tests`, and host the insert rule beside it or call it a release gate.
C. `platform_requirement_hold` has another signature and error in design 045: adopt one definition.
D. New values of closed REST enums (`waitingOn` `DECIDER` and `CLOSED`, a new reset-point kind) are not named: name or
   map them; an old-client test.

## Nit

1. The host does not depend on every scanned crate; the gates do not run it.
2. Task 044-1's gate should include Gaps 044-024 and 044-025.
3. `code_moves_total{verdict}` lacks `run_not_mutable`.
4. The domain's binding parser refuses an empty activity list.
5. Keys of `code_step_nodes` and `code_replay_checks`; a closed check on `run_events.origin`.
6. The rollout scan of graph versions has no task or test.
7. ADR-0106's test names; `code_workflow_sites.rs` in no Files line.

Counts: Blocker 0, Major 4, Nit 7
