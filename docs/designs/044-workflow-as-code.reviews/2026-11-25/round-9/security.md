# Security review of design 044, round 9 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 842f013e1e19a2d4e09617fc0abf88296275b860
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 8's Major 2 is
closed, 1 and 3 partly closed. No finding on the tenant boundary in SQL, secrets, blob and export disclosure,
injection, the runtime role, definer functions or the re-checked races.

## Major

1. The binder is never an initiator row, so a binder escapes in a child, a successor, a reset run, or after the binding
   is replaced: write the binder of the issuing session into `run_initiators`; tests for each.
2. A tenant can force cache thrashing, each miss reading up to 6 MiB: a cold read loads only the addressed step, scope
   members only when planning; whole-overlay methods answer from counts or fail closed; a numbered byte ceiling; a test.
3. An operator cannot stop a running code run at once (cooperative grace up to 3,600 s; "forced cancellation" does not
   exist): a cancellation without grace under `run:operate`; a test.
4. Making `state.schema`, `argsSchema` and `resultSchema` carriers contradicts feature 016's refusal of annotations
   there, and answers would leak classified fields: keep `CLASSIFICATION_NOT_APPLICABLE` and classify their raw
   payloads `pii` as a whole; a test.

## Nit

5. The binder refusal's scope; approvers shown the issuing identity and binder.
6. The startup privilege check must list the new append-only tables.
7. The namespace row `FOR SHARE` in every claim and completion (MultiXact growth).
8. More readers of `run_events.detail` than named; keep the server-side fields out of any projection.
9. Architecture decision 0108 should list Architecture decision 0031 decision 5 as superseded for worker queries.
10. `HistoryEvent` must carry no actor subject.

Counts: Blocker 0, Major 4, Nit 6
