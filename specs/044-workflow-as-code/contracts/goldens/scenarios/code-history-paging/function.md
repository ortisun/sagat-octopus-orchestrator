# code-history-paging

What it proves (design 044, Verification Plan): a history of more than 1,024 events read in pages.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow meter(input):
  readings = 0
  total = 0
  onSignal("reading", (p) => readings = readings + 1; total = total + p.value)
  await signal("close")
  return {meterId: input.meterId, readings: readings, total: total}
```

## What the golden shows

- 1051 visible events, 1,041 of them signal.buffered (1,040 readings and close), in three
  activations; each consumption lists at most 1,000 sequences.
- Activation 3 is taken by a cold worker: ReadHistory from sequence 1 returns a page of 1,024 events, then the
  remaining 24 (`historyPages`).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The pages are bounded by the 1,024-event count (the events are small, so the 3 MiB bound does not bind); the
  lease's first page (at most 256 KiB of the encoding) is not computed by the corpus.
