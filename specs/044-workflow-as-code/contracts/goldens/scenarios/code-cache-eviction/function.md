# code-cache-eviction

What it proves (design 044, Verification Plan): a cache eviction and a replay from the start on another worker.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow paymentFlow(input):
  requestId = uuid()
  charge = await activity("payments.charge", {orderId: input.orderId, requestId: requestId})
  fx = sideEffect(() => readRate("EURUSD"), {id: "fx-rate"})  // {pair, rate}, recorded once
  settled = await waitForEvent("payment.settled", input.orderId, {timeoutMs: 3600000, lookbackMs: 60000})
  await script("sha256:c4a45a6892a43d9557dab01899a89444a99702f4c9eb4b86e79bead5bc544d59",
               {settlementId: settled.settlementId, rate: fx.rate})
  return {orderId: input.orderId, requestId: requestId, settlementId: settled.settlementId}
```

## What the golden shows

- Worker A runs activations 1 and 2 (`cache` hit for 2: the lease's window start equals what it holds), then
  evicts the run. Worker B takes activation 3 cold, replays from sequence 1, and produces the same chain:
  the uuid comes from the seed, the side effect from its HISTORY_MARKER_RECORDED value.
- The wait is satisfied by an event (HISTORY_WAIT_OUTCOME with its payload, EXTERNAL); the script command's
  target is the artifact digest.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
