# code-deterministic-primitives

What it proves (design 044, Verification Plan): time, 64 numbers, bounded integers and 8 UUIDs of a fixed seed equal the golden.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow primitives(input):
  firstNow = now()
  numbers = [random() repeated 64 times]
  bounded = for n in [1, 2, 3, 6, 10, 1000, 2^31, 2^32]: {n, values: [randomInt(n) repeated 8 times]}
  ids = [uuid() repeated 8 times]
  await sleep(1000)
  secondNow = now()
  return {nowMs: [epochMillis(firstNow), epochMillis(secondNow)], random: numbers, randomInt: bounded, uuid: ids}
```

## What the golden shows

- The run's seed is 000102...1f (`seedHex`); the output is exactly the stream of that seed in
  `vectors/primitives.json` (the contract test compares them).
- now() is the time of the workflow_task.started at S(k): `nowUs` of each activation; the output holds it in
  milliseconds, truncated, because TypeScript's Date has milliseconds (Technical Plan 1.8). The second activation's
  time differs from the first, and a replay returns the same two values.
- Drawing numbers makes no command: only the timer and the completion are commands.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The table asks for 8 bounded integers; the function draws 8 for each of the 8 bounds of the primitives vectors so
  that the scenario and the vectors are one stream.
