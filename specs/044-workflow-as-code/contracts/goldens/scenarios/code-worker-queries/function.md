# code-worker-queries

What it proves (design 044, Verification Plan): a worker query from the committed state (pending until Task 044-29).

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow inventoryCount(input):
  counted = 0
  workerQuery("progress", (args) => {counted: counted})   // the worker-query registration of Task 044-29
  for sku in input.skus:
    await activity("inventory.count", {sku: sku})
    counted = counted + 1
  return {counted: counted}
```

## What the golden shows

- Pending: the design keeps this golden `.json.pending` until Task 044-29 builds worker queries; the `PENDING`
  file marks the scenario and the contract test skips it.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- Technical Plan 1.12 names no registration call for a worker query; `workerQuery` is a placeholder name.
