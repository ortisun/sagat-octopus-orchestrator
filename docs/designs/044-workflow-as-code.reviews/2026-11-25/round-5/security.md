# Security review of design 044, round 5 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9b64ac22698c34d847c9658b8a08b5a5c68814d2
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 4's Majors 1, 2, 3,
5, 6 and 7 are closed (reserved names, lease kinds per call, eager admission, update-key oracle, worker-query payloads,
feature 016 carriers in text); Major 4 is still open and returns as finding 1. No cross-tenant path, open metric label,
injection or runtime-role privilege was found.

## Major

1. Approval initiators (round 4, Major 4, still open): the rule covers only an activity entry with `requiresApproval`,
   not the approval command, and leaves out the principals who drive the run (cancellation requester, mover, review
   retrier, sender of an external event that wakes it). Apply it to every engine-held approval and define the initiators
   from the starter, the actors of visible external events up to the issuing activation, and the actors of
   `run.code_version_moved` and `step.retry_requested`; a test per kind.
2. A `secret` field reaching the workflow lease "as an opaque reference" is an undefined mechanism (format, binding to
   run and tenant, dereference point, target classification, schema validation, history type): cross-run read, downgrade
   to `internal`, unbuildable validation. Drop it, or specify it fully in Architecture decision 0108.

## Nit

3. `ReadBlob` for query and replay-check tokens over blobs the run's history references.
4. `QueryTask` and `RespondWorkerQuery` still say "reference"; `args` and `answer` must be class `content`.
5. Section 5.2 answers an idempotent replay before the binding, membership and tenant re-checks.
6. A move target must keep the run's workflow type.
7. A command's blob reference is not bound to its run.
8. Every registered name with the prefix `sagat.workflow` is refused; Architecture decision 0103 lacks the
   graph-publication and claim refusals and still says 4 KiB.
9. A `run:operate` reader of a receipt key used by several principals.
10. `hold_requirement` needs the definer-function properties here.
11. No task carries feature 016's new rules version; the redaction test name promises taint tracking.

Counts: Blocker 0, Major 2, Nit 9
