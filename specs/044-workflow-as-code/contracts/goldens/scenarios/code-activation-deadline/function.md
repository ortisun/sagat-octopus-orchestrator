# code-activation-deadline

What it proves (design 044, Verification Plan): an infinite loop stopped with its cause.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow feed(input):
  polled = await activity("feed.poll", {feedId: input.feedId})
  while true: pass                                           // never yields
  return polled
```

## What the golden shows

- Each attempt of activation 2 is stopped after 2 s of wall time and failed with ACTIVATION_DEADLINE, counted
  with WORKFLOW_TASK_FAILED: retried after 1, 2, 4 and 8 s, then the run is under review (code
  WORKFLOW_TASK_FAILED) after the fifth consecutive failure.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The review raised by counted failures carries the code WORKFLOW_TASK_FAILED whatever the counted cause; each
  attempt's own cause is ACTIVATION_DEADLINE.
