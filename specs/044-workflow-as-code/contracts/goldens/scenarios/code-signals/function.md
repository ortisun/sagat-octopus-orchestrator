# code-signals

What it proves (design 044, Verification Plan): a signal before the function waits, a repeated key, an undeclared name refused.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow cart(input):
  items = []
  onSignal("item.added", (p) => items.push(p.sku))
  await activity("cart.open", {cartId: input.cartId})
  payment = await signal("checkout")
  return {cartId: input.cartId, items: items, checkout: payment}
```

## What the golden shows

- item.added is buffered at sequence 2, before the first workflow task: activation 1 delivers it to the handler and
  the SDK consumes it (`consume_signals`, after the function's own command).
- The second item.added is sent twice with one idempotency key: one signal.buffered (`requests`).
- coupon.applied is not declared: refused 409 with SIGNAL_NOT_DECLARED, nothing journaled.
- Activation 3 consumes two names in one command, names in ascending UTF-16 order (checkout before item.added).

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- The HTTP status of an accepted signal is today's 202 of `POST /runs/{runId}/signals/{signalName}`.
