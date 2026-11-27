# Targeted re-read 2 of design 052: correctness (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 58b6fd20e8723d36fdac8be920b5cf5ca13dd3cb
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`, at d734e026),
then `reread-1-correctness.md` (at 9b3ec898), whose five Majors and three Nits are confirmed resolved here (the
arithmetic rechecked: 1.425, 0.995, 11/1024, 1/64). Not a full review. The reviewer read files only and ran nothing;
items marked plausible rest on its reading of the code.

No Blocker.

## Major

1. The spread switches itself off: a spread round meets no contention, which clears the flag, so every other round
   collides again. Fix: the flag lapses only 1 s after the last contended probe; the one-poller test asserts a third
   round stays spread.
2. (Plausible) A round can last for ever: the decision is taken when the in-flight entry is created, which under
   overlapping streams never empties, so the spread never turns on or never turns off. Fix: the first poll entering more
   than 1 s after the round's decision decides again; one switch-over overlap accepted and stated; a test with polls
   that never drain.
3. (Plausible) "Not comparable" lets a regression through: both builds already run the merge base's scenario, so a pull
   request that slows the claim and edits one scenario line passes. Fix: "not comparable" only when the merge base's
   scenario cannot build or run against the head; otherwise compare as usual; a test with both together.

## Nit

1. A lease's debit can take a parked peer's permit; hand an added permit straight to a parked waiter under the mutex.
2. No rule for a process missing from `serving_instances` (`n` 0); offset 0 then, with a unit case.
3. In Success criterion 052-005's configuration (eight pollers of `wanted` 16, a window of 64) the spread has no effect;
   state when it acts and measure it on and off with two pollers per instance.

Counts: Blocker 0, Major 3, Nit 3

The three Majors and three Nits were fixed at the next commit, with the architecture confirmation beside it.
