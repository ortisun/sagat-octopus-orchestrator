# Collection active-drain conformance draft

This directory is a hand-authored proposal. Nothing here was blessed or run, and nothing in the
repository was changed.

## Why the barrier is deterministic enough to try

The scenario starts only the `holder` worker, waits for the public journal's `step.started` event,
then starts `failer`. The holder has `maxInflight: 1` and its handler is still blocked in `hang`, so
it cannot lease the second task. The failer therefore supplies the permanent failure while the
first handler is active. This uses existing behaviors and generic operations documented in
`tests/contract/sdk-conformance/README.md`: `hang`, `fails`, `startWorker`, `awaitJournal`,
`awaitWorkerLog`, and `waitForCompletion`.

There is one implementation-level assumption to review before adopting the golden: the holder is
expected to receive iteration 0. The PostgreSQL candidate query orders forward work by `ready_at`
(`crates/infra-postgres/src/workers.rs`, `CLAIMABLE_PATHS`), and each collection admission writes
the transition time into `steps.ready_at` (`crates/infra-postgres/src/primitives.rs`). Admissions are
separate scheduler actions. This makes iteration 0 the older ready task in the current runtime, but
the query has no instance-path tie break if two `ready_at` values are equal. The public SDK contract
does not separately promise an index tie break. If an unblessed run ever swaps holder/failer indices,
do not normalize the index away and do not accept alternate goldens: add an explicit common
handler-entry barrier plus index-selected behavior to the five conformance runners, or add the
instance path as the final server candidate-order key and cover that server rule.

## Expected persisted facts

- `scopeEvidence: "summary"` reads the journal and must show `admitted: [0,1]`,
  `checkpointed: []`, `peakActive: 2`, a failed drain and failed finish with `NO_STOCK`.
  Iteration 2 is absent, so admission stopped. Summary deliberately omits returned/drained event
  ordering for concurrent iterations.
- The targeted public journal waits require `step.failed`, `collection.drain_started`,
  `step.cancel_requested`, and `step.cancelled`. They prove presence without pinning unrelated total
  journal order.
- The graph projection must leave `done` `NOT_STARTED`, mark `lines` `FAILED/NO_STOCK`, mark one
  scoped `work` `CANCELLED` and the other `FAILED/NO_STOCK`, and contain no third iteration or return.
- `sourceEvidence: true` has the existing shape
  `{runId,status,stopCode,collection:{itemCount,fromBlob,blobReads}}`; it must report
  `FAILED`, `NO_STOCK`, `3`, `false`, and `0`.
- Separate worker logs contain one entry each. Their schema is the existing
  `{activity,kind,node,attempt,input,taskKind,targetNode,targetIdempotencyKey,idempotencyKey,
  traceparentPresent,traceId,steps,deadlineAhead,abortedReason,outcome,instance,scopeInput}`.
  The failer entry is `failed` with no abort reason. The holder entry is `cancelled`.

The drafted holder abort reason is `run-cancelled` because current worker hosts map the protocol's
`cancel_requested` heartbeat flag to that public reason, and the reviewed `loop-cancel` golden has
the same value for a scoped active handler. The server currently does not expose a distinct
collection-drain cancellation reason. Treat the unblessed run as the final check.

## Oracle traps avoided

- No `events` dump: the exact position of the second admission relative to the holder lease, and the
  order in which concurrent iterations drain, are not the product contract.
- No `holdUntilLogged`: that counter advances only when a handler ends, so it cannot prove an active
  sibling.
- No compensation activity: WP-04 promises no order between collection iterations' compensations.
- Workers stop only after terminal completion, preventing `shutdown` from masquerading as the drain
  cancellation.
- `scopeEvidence` is `summary`, not `ordered`, because two iterations overlap.

## Intended repository paths after review

- `tests/contract/sdk-conformance/scenarios/collection-fail-fast-drain.json`
- `tests/contract/sdk-conformance/expected/collection-fail-fast-drain.workflows.json`
- `tests/contract/sdk-conformance/expected/collection-fail-fast-drain.json`
- scenario description in `tests/contract/sdk-conformance/README.md`
- WP-04 trace updates in `specs/014-workflow-primitives/contracts/conformance.md`,
  `specs/014-workflow-primitives/research.md`, and `docs/sdk-compatibility.md`

The temporary runtime expected file is named `collection-fail-fast-drain.expected.json` only to keep
it visually distinct from the scenario in this draft directory. Rename it to
`expected/collection-fail-fast-drain.json` when applying it.
