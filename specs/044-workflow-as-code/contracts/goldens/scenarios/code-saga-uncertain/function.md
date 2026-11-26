# code-saga-uncertain

What it proves (design 044, Verification Plan): an unknown outcome reconciled applied, and one left for review.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow settlement(input):
  await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
      charge = await s.activity("payments.charge", {invoiceId: input.invoiceId, amountCents: input.amountCents})
                                                         // envelope: reconcileWith payments.lookup
      await s.activity("ledger.post", {invoiceId: input.invoiceId, chargeId: charge.chargeId}))
                                                         // envelope: reconcileWith ledger.lookup
  return {invoiceId: input.invoiceId}
```

## What the golden shows

- payments.charge reports an unknown outcome; its lookup says applied, so c-2 completes with the lookup's
  output: one visible STEP_OUTCOME, nothing of the reconciliation.
- ledger.post's lookup is not conclusive: c-3 waits for a review, nothing is visible, and the run stays open
  with the function waiting (the history ends after activation 2's records).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The kinds in `invisibleJournal` for feature 037's reconciliation are indicative.
