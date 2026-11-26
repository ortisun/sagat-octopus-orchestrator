# Correctness review of design 044, round 9 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 842f013e1e19a2d4e09617fc0abf88296275b860
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Both round-8 Majors are
closed.

## Major

1. A value the SDK sends as a blob puts a random blob identifier into the hashed body, so no replay reproduces the
   chain and a re-emitted deferral differs: hash the content digest and size instead of the reference; tests.
2. Continue-as-new is accepted for a run that has no continuation chain (its start version had no policy), and the
   pass never finds it: refuse unless the run heads an open chain; the chain's limits govern the successor; tests.

## Nit

3. Section 9 still says `version: active` is resolved at the handoff.
4. Scripts and gated activities are not named among the obligations.
5. The 56,320 ceiling can be exceeded when outcomes commit during the review.
6. Update commands have no target, and keys are scoped by principal.
7. When the overlay cache is filled.
8. Two rules on when the SDK issues a terminal command inside an open scope.
9. A continue-as-new input that is not an object, or a blob.
10. A move's fit check covers only envelope entries, not signals, updates and the state schema.
11. A lease within two events of the window's bound can never complete.

Counts: Blocker 0, Major 2, Nit 9
