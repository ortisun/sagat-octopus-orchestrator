# Approval confirmation of design 052: correctness (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d770bb0c159f12d11e62f45cc4f327cb95a920df
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`, at d734e026),
then `reread-1-correctness.md` and `reread-2-correctness.md`, whose findings are all confirmed resolved here (the
flag's lapse by time, the round's second decision, the permit hand-off, rank 0, the second configuration). Not a full
review. The reviewer read files only and ran nothing.

No Blocker.

## Major

1. (Plausible) The gate passes a head that breaks the scenario: a run failure or timeout against the head counts as
   "cannot run", and a compile break of the scenario's internal interfaces too, so a ten-times slower or hanging claim
   passes with "not comparable", and the pilot re-run compares nothing. Fix: a run failure or timeout fails the gate;
   only a compile failure is "not comparable", after trying the head's scenario on both builds, then with a reviewer's
   recorded acknowledgement; a test where the head's run times out.

## Nit

1. The spread's boundary: it keeps processes apart while each asks for at most `len / n`, and reduces overlap up to
   `len`.
2. Statements per lease cannot be read from `worker_poll_statements_total` alone; count the claim transactions'
   statements too, or take a `pg_stat_statements` delta in Task 052-9.
3. A stream registered on two keys can be handed two permits; hand a permit only to a stream not already handed one;
   the check before the park and the marking as parked are one critical section, with a barrier there.
4. The check-before-park barrier test belongs also in `integration_claim_transition_efficiency`, which runs for changes
   of `wake.rs` and `service.rs`.

Counts: Blocker 0, Major 1, Nit 4
