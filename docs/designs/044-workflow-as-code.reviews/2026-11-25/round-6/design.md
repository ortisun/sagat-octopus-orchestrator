# egm-review Stage 7 design lens, targeted re-read, round 6 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: b5c23b83c9a63c8ed42f15b4cdf881ecb683dd4e
Reviewer: egm-design, feature-spec standard, FeatureRange from the start to the Detailed Implementation Plan
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 5's findings,
1 to 7 and 9 to 13 are closed and 8 partly closed. The saga and stop-drain claims were checked against the code.

## Major

1. Code scopes have no step row, but `sagas.rs` gates every saga decision on a saga step waiting (`running`, `owned`,
   `advance`): a stop during code compensation would stop the compensations (principle III). Give each scope a step row
   or name the code arms; two tests.
2. The function is never given its input: `run.started` is not visible, `HistoryEvent` and `WorkflowTaskLease` carry
   no input, `S(0)` is undefined, a successor's carried input and state have no carrier.
3. A reset of a run with an open code update: the source tells the client `RUN_ENDED` while the reset run replays the
   handler and completes it; refuse such a point, rebuild the transitions, state code updates are not reapplied; a test.
4. A completion beyond the run's 4 MiB or the tenant's content quota has no cause and no way out, and
   `stop_for_storage_quota` would end the run: a `RETAINED_BYTES_LIMIT` review with a closing window; a tenant quota
   refusal counted with backoff.
5. `code_can_act_count` is a hand-kept counter with many writers and no repair: derive it in the typed append from a
   domain predicate with a parity test, or compute it; all-or-null check and `verify_run`.

## Nit

6. "First commit wins" against the completion's deadline check: judge by the database clock.
7. Section 9 contradicts Requirements 044-019 and 044-031 for continue-as-new with an open scope.
8. Overlay rows do not record the accepting version or the secrets.
9. The namespace pause's effects on leases, grace and worker queries.
10. The append's wake statement inside a completion is outside the statement budget.
11. The Overview and the Temporal table overclaim the chain for side-effect values.
12. `is_structured` is missing from the method list.
13. Cancellation of a run already in a `Completed` drain; a child cancel command through `request_cancellation`.

Counts: Blocker 0, Major 5, Nit 8
