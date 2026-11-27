# code-declared-state

What it proves (design 044, Verification Plan): declared queries with every worker stopped, and on the ended run.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow fulfilment(input):
  setState({status: "reserving", items: 0})
  reserved = await activity("inventory.reserve", {orderId: input.orderId})
  setState({status: "charging", items: reserved.items})
  charged = await activity("payments.charge", {orderId: input.orderId})
  setState({status: "done", items: reserved.items, receiptId: charged.receiptId})
  return {orderId: input.orderId, receiptId: charged.receiptId}
```

## What the golden shows

- set_state is chained with the state itself as its body (always inline).
- The engine answers the declared query `status` (expression `state.status`) from the committed state with no
  worker running, and after the run ended (`expectations`).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The SDK sends the declared state once per completion, after the function's other commands and before a terminal
  command (the last setState of the activation wins, Requirement 044-016).
