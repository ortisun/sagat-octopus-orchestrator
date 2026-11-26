# Targeted re-read of design 046 for approval: correctness (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cd82af9376ce4a8d40196f11541a3ca1ac754207
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The fourth and last targeted
correctness re-read after review round 3 (approval rule of Owner decision 2026-11-25-2); the others are
`reread-1-correctness.md` (Major 5, Nit 4), `reread-2-correctness.md` (Major 1, Nit 3) and `reread-3-correctness.md`
(Major 1, Nit 2), every finding fixed before the next pass. The reviewer read the diff since `1f12ee02`, the design,
the data model and contracts, and `crates/infra-postgres/src/pool.rs`; it ran nothing.

## The previous findings

1. Answered: the general key is a partial index `WHERE phase <> 'BACKFILL'`, and `BACKFILL` has its own key over live
   rows. 2. Answered: the sweep cancels only expired `RESOLVE` and `INSTALL` jobs; an expired backfill lease is retried.
3. Answered: both retryable `503` codes on the publication row and in a code table.

## New findings

1. Major (plausible). The barrier's `SHARE` lock and the switch's exclusive snapshot lock both queue new takers behind
   them while they wait, so every publication, resolve completion and finish of that ecosystem waits too; every pool's
   default `statement_timeout` is 5 s (`crates/infra-postgres/src/pool.rs:62`). One writer open for 20 s (an idle
   transaction, capped at 60 s, `pool.rs:63`) or an anti-wraparound autovacuum (`SHARE UPDATE EXCLUSIVE`, not cancelled)
   makes every queued writer cancel at 5 s (`57014`), on every retry; the design does not say what a client then gets (a
   `500`, probably), and if the import pool keeps the 5 s default the stated 30 s is never reached. Fix: a `lock_timeout`
   below the runtime statement timeout (2 s, as `20270114000000_trigger_handover.up.sql`); map a queued writer's `57014`
   or `55P03` to the retryable path (a job fails retryable, a publication answers `503 SCRIPT_POLICY_BUSY`); a test with
   a writer transaction held open.
2. Nit (plausible). "For every artifact the range evaluation matched": a row read while its version was `BUILDING` has
   no artifact digest; if the switch uses digests from the application's read it misses an artifact finished in between,
   and no `SNAPSHOT` row is written since the row was evaluated. Fix: pass the matched pairs and join them to
   `artifact_digest` under the lock.
3. Nit. A re-evaluation page covering npm and pypi takes two snapshot locks in no stated order; two pages in opposite
   orders with both switches waiting can deadlock (PostgreSQL aborts one: a retry, not a hang). Fix: ecosystem-name order.

Counts: Blocker 0, Major 1, Nit 2

## The author's answers

1. Settled by Owner decision 2026-11-24-314 (delegate) and fixed where the build reads it: the data model's lock order
   now gives the barrier and the switch a 2 s `lock_timeout` with retries and the alert, and sends a decision cancelled
   while queued (`57014`, `55P03`) to the retryable path (`503 SCRIPT_POLICY_BUSY` for a publication, in `errors.md`);
   follow-up Task 046-28 restates the design's two sentences ("30 s", "the same timeout") and names the held-writer test,
   before the build of Task 046-5. 2. Fixed in the data model's switch (pairs joined to `artifact_digest` under the lock);
   the design's sentence is restated by the same follow-up task. 3. Fixed in the data model (ecosystem-name order).
