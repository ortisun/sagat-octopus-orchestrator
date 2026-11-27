# code-continue-as-new

What it proves (design 044, Verification Plan): a deferred continue-as-new while an activity is open, a losing timer cancelled, transferred state, version active, a moved run continuing on its current version.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow subscription(input, carried):
  if input.cycle >= 2: return {cycles: input.cycle, charged: carried.charged}
  if input.wait: await sleep(1000); continueAsNew({cycle: 2, amountCents: input.amountCents})   // run "moved"
  activity("audit.write", {cycle: input.cycle})              // not awaited
  charge = activity("billing.charge", {cycle: input.cycle, amountCents: input.amountCents})
  deadline = sleep(30000)
  winner = await race(charge, deadline)
  cancel(deadline)                                           // the losing timer
  continueAsNew({cycle: input.cycle + 1, amountCents: input.amountCents},
                {version: "active", state: {charged: [winner.charged]}})
```

## Runs

- this directory: cycle 1: the deferred continue-as-new and the cancelled timer.
- `successor/`: cycle 2 on the active version (2): run.started carries the input, the state and the predecessor.
- `moved/`: a run moved from version 1 to 2 that continues as new with version "same".

## What the golden shows

- Activation 2 cancels the timer c-3 (command 4, `timer_cancel`) and makes continue-as-new while audit.write (c-1)
  is open: deferred with OUTSTANDING_OBLIGATIONS, no number; the deferral is in its workflow_task.completed.
- When c-1 completes, activation 3 re-emits it unchanged; it takes number 5 and is chained then.
- The continue-as-new body holds its input and state inline and `version` "active"; the successor's
  run.started carries `carriedState` and `predecessorRunId`; the successor has a new seed.
- The moved run's continue-as-new has `version` "same" and no state (null in the body).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- Technical Plan 1.12 names no call that cancels a timer or a step; `cancel(handle)` stands for the SDK's
  cancellation of its own step (the commands timer_cancel and step_cancel exist).
- A timer cancellation is recorded by one visible event, its command record (journal step.cancelled); no
  separate step outcome is written for it.
- run.code_version_moved is not visible (it is not in the visible list of Technical Plan 4.2).
