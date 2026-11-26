# code-gated-activity

What it proves (design 044, Verification Plan): an activity whose entry requires approval, approved and rejected.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow payouts(input):                       // envelope: payouts.send has requiresApproval "large-payouts"
  first = activity("payouts.send", {payoutId: "p-1", amountCents: 250000})
  second = activity("payouts.send", {payoutId: "p-2", amountCents: 990000})
  [a, b] = await allSettled(first, second)
  refund = await approval("refund-review", {refundId: "r-9", amountCents: 1200})
  return {batchId: input.batchId, sent: ["p-1"], rejected: ["p-2"], refund: refund}
```

## What the golden shows

- The lease lists `gatedEntries` ["payouts.send"]: the SDK keeps those inputs inline.
- The engine holds both steps for approval (not visible); c-1 is approved and completes, c-2 is rejected and
  fails with APPROVAL_REJECTED, which is all the function sees.
- An approval command (command 3, target the policy name, node c-3) and its HISTORY_APPROVAL_DECIDED.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
