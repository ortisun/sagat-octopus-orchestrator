# code-update-with-start

What it proves (design 044, Verification Plan): Update-with-Start on a code version.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow basket(input):
  items = []
  onUpdate("addItem",
      validator: (args) => if items contains args.sku: throw ValidationFailed,
      handler: (args) => items.push(args.sku); return {items: length(items)})
  await sleep(600000)
  return {basketId: input.basketId, items: items}
```

## What the golden shows

- run.started (1) and update.admitted (2) are written by the creating transaction; activation 1 applies both.
- run.started starts the function, which registers its handler and sleeps (command 1); update.admitted then
  runs the validator (command 2, accept) and the synchronous handler (command 3, its result).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
