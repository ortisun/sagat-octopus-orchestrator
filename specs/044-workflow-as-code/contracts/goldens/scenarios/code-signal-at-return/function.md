# code-signal-at-return

What it proves (design 044, Verification Plan): a signal that arrives in the window of the activation in which the function returns is applied before the terminal command.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow report(input):
  notes = []
  onSignal("note", (p) => notes.push(p.text))
  built = await activity("report.build", {reportId: input.reportId})
  return {reportId: input.reportId, url: built.url, notes: copy(notes)}
```

## What the golden shows

- Activation 2: the handler takes note a, the function returns, the handler takes note b, then the SDK sends
  `consume_signals` (both sequences) and, last, `complete`.
- Note c commits after the workflow_task.started of task 2: the completion defers `complete` with UNHANDLED_EVENTS
  (its workflow_task.completed carries the deferral; the command takes no number).
- Activation 3 consumes note c and re-emits the same `complete` (the same body digest), which takes number 4.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
