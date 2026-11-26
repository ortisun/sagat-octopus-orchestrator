# code-updates

What it proves (design 044, Verification Plan): validator, accept, reject, an asynchronous handler with an activity, 200, 202 with acceptPending.

## Reference function

Pseudo-code with the API names of Technical Plan 1.12.

```text
workflow creditLine(input):
  limit = input.limit
  onUpdate("raiseLimit",
      validator: (args) => if args.amount <= 0: throw ValidationFailed,
      handler: async (args) =>
          applied = await activity("limits.apply", {accountId: input.accountId, amount: args.amount})
          limit = applied.limit
          return {limit: applied.limit})
  await sleep(3600000)
  return {accountId: input.accountId, limit: limit}
```

## What the golden shows

- Two updates admitted while the function sleeps: the first passes its validator (`update_accept`, target the
  decimal sequence of its update.admitted) and its handler schedules limits.apply; the second fails it
  (`update_reject`). Each decision is made when its update.admitted is applied, before the next event.
- The third update is sent with acceptPending: the client gets 202 ACCEPTED while its handler waits for c-6.
- Each handler's return is `update_complete` with the result as a descriptor; the first client gets 200.

## Assumptions

- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,
  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with
  indicative kinds in `commands.json` under `invisibleJournal`.
- A validator's rejection uses VALIDATION_FAILED, of the closed list of rejection reasons (VALIDATION_FAILED,
  and the engine's RUN_ENDED and RUN_CONTINUED).
- The REST answer of a rejected update (status and body) belongs to the REST contract; `requests` states only the
  outcome.
