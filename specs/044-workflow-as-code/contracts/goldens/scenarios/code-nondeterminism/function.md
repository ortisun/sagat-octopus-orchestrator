# code-nondeterminism

What it proves (design 044, Verification Plan): a changed function on replay: the report and the review.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow notifyOrder(input):                                 // version 1, which wrote the history
  order = await activity("order.load", {orderId: input.orderId})
  await activity("email.send", {orderId: input.orderId, to: order.email})
  return {orderId: input.orderId}

workflow notifyOrder(input):                                 // the second version, replayed by worker B
  order = await activity("order.load", {orderId: input.orderId})
  await activity("sms.send", {orderId: input.orderId, to: order.email})
  return {orderId: input.orderId}

workflow bookTrip(input):                                    // run "scope-moved", version 1
  saga({}, async (s) => {})                                  // s-1, empty: it closes after the next call
  await activity("hotel.book", {tripId: input.tripId})       // outside any scope

workflow bookTrip(input):                                    // its second version, replayed by worker B
  await saga({}, async (s) =>                                // s-1
      await s.activity("hotel.book", {tripId: input.tripId}))           // now in s-1
```

## Runs

- this directory: a changed target: email.send became sms.send.
- `scope-moved/`: a changed saga scope: hotel.book moved into the saga s-1, its kind and target unchanged.

## What the golden shows

- Worker A ran activations 1 and 2 with the first version. Worker B, cold, takes task 3, replays from journal
  sequence 1 and makes sms.send where the history has the record of email.send at command 2.
- The report (FailWorkflowTask, cause NONDETERMINISM): command number 2, actual kind COMMAND_ACTIVITY, actual
  target sms.send, the code digest; no payload. The run goes under review at once.
- scope-moved: the record of command 2 has no saga_scope (HistoryCommandRecord.saga_scope empty) and worker B makes
  hotel.book in s-1. The report names command 2, kind COMMAND_ACTIVITY, target hotel.book and actual_saga_scope
  s-1 (Decision 044-244); a report whose scope is outside `^s-[1-9][0-9]{0,9}$` has it dropped by the server.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The code digest names the code, so the second version is registered under the run's digest by the conformance
  harness only: it stands for a function that is not deterministic. The report carries the run's digest.
