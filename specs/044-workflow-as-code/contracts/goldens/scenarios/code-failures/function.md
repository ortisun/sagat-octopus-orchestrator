# code-failures

What it proves (design 044, Verification Plan): a workflow failure ends the run; an uncaught exception puts it under review after five consecutive failed workflow tasks.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow checkout(input):
  try: await activity("payments.charge", {orderId: input.orderId, amountCents: input.amountCents})
  catch ActivityFailure: throw WorkflowFailure("PAYMENT_DECLINED", "The card was declined")
  return {orderId: input.orderId}

workflow invoice(input):                                     // run "uncaught-exception"
  loaded = await activity("invoice.load", {invoiceId: input.invoiceId})
  if loaded.invoice == null: throw InvoiceMissing()          // not the SDK's WorkflowFailure
  return loaded.invoice
```

## Runs

- this directory: WorkflowFailure: the `fail` command, the run ends FAILED with its code.
- `uncaught-exception/`: five failed workflow tasks, then the review.

## What the golden shows

- `fail` hashes its code and the descriptor of the message as a JSON string; the message is never in the history.
- uncaught-exception: each attempt is a visible workflow_task.started followed by an invisible
  workflow_task.failed (cause WORKFLOW_TASK_FAILED, exception type InvoiceMissing); the next attempt starts after
  1, 2, 4 and 8 s; after the fifth the run is under review (no completion, the chain stays at command 1).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
