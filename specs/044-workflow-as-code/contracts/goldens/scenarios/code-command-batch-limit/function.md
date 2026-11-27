# code-command-batch-limit

What it proves (design 044, Verification Plan): an activation that would make a 257th command catches CommandBatchLimitExceeded, live and in replay.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow notifyAll(input):
  started = []
  overflowFrom = null
  try:
    for i in 1 .. input.recipients: started.push(activity("notify.send", {recipient: i}))
  catch CommandBatchLimitExceeded:
    overflowFrom = length(started) + 1                       // 257
  await all(started)
  await child("notify-fanout", {from: overflowFrom, to: input.recipients}, {cancelPolicy: "propagate"})
  return {sent: length(started), delegated: input.recipients - overflowFrom + 1}
```

## What the golden shows

- Activation 1 makes 256 commands; the 257th call throws CommandBatchLimitExceeded into the function, which
  catches it (`thrown`). A replay throws at the same call, so the chain is the same.
- The rest moves to a graph child in the next activation (command 257, a new activation's share).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
