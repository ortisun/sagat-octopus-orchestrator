# code-blob-values

What it proves (design 044, Verification Plan): a blob-carried activity input replayed cold, and a deferred blob output re-emitted by another worker.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow warehouseImport(input):
  notes = []
  onSignal("audit.note", (p) => notes.push(p.text))
  rows = {rows: [{i: i, label: "row-" + i} for i in 0 .. input.rowCount - 1]}
  imported = await activity("warehouse.import", rows, {timeoutMs: 600000})
  return {rows: [{i: i, label: "out-" + i} for i in 0 .. input.rowCount - 1]}
```

## What the golden shows

- Both values are larger than the 320 KiB message, so they travel as blobs; `valueBlob` gives the generator
  that rebuilds each and its descriptor, which is all the chain hashes.
- The completion of activation 2 is deferred (UNHANDLED_EVENTS: audit.note arrived after its start) with its
  output blob held; worker B re-emits it naming that blob by its descriptor and it is accepted as command 3.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The values never appear in the corpus; the contract test rebuilds them from `valueBlob.generator`
  (`{"rows": [{"i": i, "label": label + i}, ...]}`) and checks the descriptors.
