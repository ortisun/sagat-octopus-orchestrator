# code-patch-and-move

What it proves (design 044, Verification Plan): a patched version moved; a run that does not match stays.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow purchase(input):                                    // version 1
  quote = await activity("quote.fetch", {orderId: input.orderId})
  await activity("order.place", {orderId: input.orderId, quoteId: quote.quoteId})
  await sleep(86400000)
  return {orderId: input.orderId, status: "placed"}

workflow purchase(input):                                    // version 2
  quote = await activity("quote.fetch", {orderId: input.orderId})
  if patched("use-fraud-check"):
    await activity("fraud.check", {orderId: input.orderId, quoteId: quote.quoteId})
  await activity("order.place", {orderId: input.orderId, quoteId: quote.quoteId})
  await activity("notify.send", {orderId: input.orderId})    // a change without a patch
  await sleep(86400000)
  return {orderId: input.orderId, status: "placed"}
```

## Runs

- this directory: moved after activation 1, then run on version 2: the patch marker is recorded the first time.
- `unmatched/`: at activation 3 on version 1: the replay check finds the first difference at command 3.

## What the golden shows

- main: the move is applied (`expectations.move`); activation 2 on version 2 records the patch marker (command 2,
  HISTORY_MARKER_RECORDED with its command number) and schedules fraud.check.
- unmatched: the dry-run or move verdict is REPLAY_MISMATCH with first difference 3; nothing moves.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- run.code_version_moved is not visible; `codeVersion` of each activation says which version ran it.
