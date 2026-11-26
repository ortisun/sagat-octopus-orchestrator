# code-run-lifecycle

What it proves (design 044, Verification Plan): start, two activities with all(), a timer, completion; the journal and the observed graph.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow order(input):
  [reservation, authorization] = await all(
      activity("inventory.reserve", {sku: input.sku, quantity: input.quantity},
               {retry: {maxAttempts: 3, initialDelayMs: 1000, strategy: "exponential"}, timeoutMs: 30000}),
      activity("payments.authorize", {orderId: input.orderId, amountCents: input.amountCents}, {timeoutMs: 30000}))
  await sleep(60000)
  return {orderId: input.orderId, reservationId: reservation.reservationId,
          authorizationId: authorization.authorizationId}
```

## What the golden shows

- Activation 1 makes commands 1 and 2. c-2 completes first; activation 2 sees only that outcome and makes no command
  (an empty completion still moves S); c-1 completes while task 2 is started, so it belongs to activation 3.
- Activation 3 starts the timer c-3 (a timer's target is its own step); activation 4 completes the run.
- `observedGraph`: c-1 and c-2 issued at S(1) with no dependency, c-3 issued after both outcomes (its issuedSeq is
  at or after their outcomeSeq), which is the observed graph the Studio draws.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
