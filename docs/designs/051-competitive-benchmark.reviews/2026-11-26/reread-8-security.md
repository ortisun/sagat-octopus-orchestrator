# Targeted re-read 8 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 11b5f3093c972e1da022d17655640bff7bcfc6ba
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Filed first as the
approval confirmation, then refiled as a re-read when the correctness confirmation at the same commit
(`reread-2-correctness.md`) found Blockers whose fixes changed the text. A re-read in the approval run under Owner
decision 2026-11-25-2: one full round (`security-full.md`), then the fixes
re-read in `reread-1-security.md` to `reread-7-security.md`, every finding of each fixed before the next. Not a full
review. The reviewer read files only and ran nothing.

Re-read 7's Nit is resolved: the per-rung `pg_stat_statements` snapshots stay in the host-only directory, which no
container can write, and only their per-table differences (`queryid`, table, `calls`, `total_exec_time`, never `query`)
are uploaded; the raw-data allowlist names the snapshots among what is never uploaded. The adopted remark holds: each
scan fails the turn when `stats_reset` is not the time of the turn's single reset, which the product roles cannot
cause, not being superusers. The earlier conclusions stand: the workflow's token, secret, cache, pinning, input and ref
rules; Task 051-0's release-workflow hardening before the first dispatch; the compose rules with the single
sandbox-runner exception; the `docker run` and `docker create` rule; the `openat2` reads beneath the record root; the
upload rule. Considered and not raised: a refused `SET` or `set_config` printed to the log carries its values as
placeholders, so no secret reaches the log.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
