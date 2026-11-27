# code-cancellation

What it proves (design 044, Verification Plan): cooperative cancellation, the grace that ends the run, an acknowledgement while a saga scope is open, and the cancellation of the function's own wait and approval.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow reportRender(input):
  if input.idle: await sleep(604800000); return {}            // run "grace-expires": never looks at cancellation
  render = activity("report.render", {reportId: input.reportId})
  await race(render, cancellation.promise)
  if cancellation.requested:
    cancel(render)
    await activity("report.cleanup", {reportId: input.reportId})
    acknowledgeCancellation()

workflow trip(input):                                         // run "acknowledge-in-saga"
  await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
      await s.activity("trip.hold", {tripId: input.tripId})   // envelope: compensateWith trip.release
      await race(sleep(86400000), cancellation.promise)
      if cancellation.requested: acknowledgeCancellation())

workflow refundReview(input):                                 // run "own-steps"
  confirmed = waitForEvent("refund.confirmed", input.refundId, {timeoutMs: 86400000})
  review = approval("refund-review", {refundId: input.refundId, amountCents: 4200})
  await race(confirmed, review, cancellation.promise)
  if cancellation.requested:
    cancel(confirmed); cancel(review)
    await allSettled(confirmed, review)                       // each rejects with CommandCancelled
    acknowledgeCancellation()
```

## Runs

- this directory: a cooperative cancellation: the step cancel, a cleanup activity, then the acknowledgement.
- `grace-expires/`: the function ignores the request; the deadline (grace 5,000 ms) ends the run CANCELLED.
- `acknowledge-in-saga/`: the acknowledgement while a scope is open: abort, deferral, compensation, acceptance.
- `own-steps/`: the function cancels its own wait and approval, then acknowledges.

## What the golden shows

- run.cancel_requested is visible and waking (EXTERNAL) with its grace and the origin REQUEST.
- main: `step_cancel` of the leased c-1 (journal step.cancel_requested); the engine's CANCELLED outcome follows.
- grace-expires: no answer; the deadline pass stops the run; the stop drain's cancellation of the timer is
  journaled after the stop began, and no workflow task follows.
- acknowledge-in-saga: `saga_abort` then `acknowledge_cancellation`, deferred with OUTSTANDING_OBLIGATIONS while
  the scope aborts; once it is compensated the acknowledgement is re-emitted and accepted (number 5).
- acknowledge-in-saga: the record of the scope's activity carries its scope (`sagaScope` s-1, Decision 044-244).
- own-steps: a cancelled wait and a cancelled approval are each a HISTORY_STEP_OUTCOME with STEP_OUTCOME_CANCELLED
  and the origin EFFECT, appended by the completion that cancelled them (Decision 044-245), which also withdraws
  the open approval request. The effects wake the next workflow task.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The acknowledgement's record is visible (HISTORY_COMMAND_RECORDED) although its journal event is
  run.stop_requested, which worker.proto calls not visible for that origin (see the report on the corpus).
- `cancel(handle)` stands for the SDK's cancellation of its own step (`requestCancellation` in the TypeScript
  runtime).
