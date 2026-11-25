# egm-review Stage 7 design lens, targeted re-read, round 5 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9b64ac22698c34d847c9658b8a08b5a5c68814d2
Reviewer: egm-design, feature-spec standard, FeatureRange 1-1915
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All five Majors of round 4
are closed (saga accessors, signal consumption, the Python importer, deferred terminal commands, the reconciler).

## Major

1. A code update has no outcome when the run ends or continues: an admitted or accepted update stays non-terminal for
   ever; fail every non-terminal update in the stop drain and the handoff, or defer the terminal command; barrier tests.

## Nit

2. The open-saga command's step row and the code variant of `saga.entered`.
3. Consumption has two carriers; the command's target and body.
4. Lazy imports during an activation; the module set is closed at load.
5. The reconciler's index and the column the 60 s test reads.
6. Eager leases counted as one statement where a lease costs 38.
7. Text still requiring `signal-inbox-v1` for every code publication.
8. Side-effect values reach the worker; limit the chain's claim.
9. Closing a saga with a member in flight; a command naming a closed scope.
10. One origin per `append_events` call; put it on the row.
11. The remaining `WorkflowDefinition` methods (`declared_secrets`, `script_digests`, `projected_state` and others).
12. On reset, signals buffered before the cut and not consumed.
13. A `run:operate` reader of a receipt key shared by several principals.

Counts: Blocker 0, Major 1, Nit 12
