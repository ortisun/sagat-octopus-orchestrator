# Targeted re-read 4 of design 051: correctness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: e52e9e73cef7a861fb33bb232b7cec407c311c35
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Run as the approval
confirmation; filed as a re-read because its findings changed the text. Every finding of `reread-3-correctness.md` is
resolved or moot under the commit check. The mechanism holds against PostgreSQL's commit sequence (deferred triggers
before the pre-commit callbacks, a function's `SET` clause ended by commit, autovacuum lowering the setting only to
`local`); this platform passes (no `synchronous_commit`, `commit_delay` or `UNLOGGED` in its crates or migrations, its
`set_config` calls only `lock_timeout` and `statement_timeout`, its migrations runnable by a non-superuser owner);
Temporal is judged to pass, Windmill cannot be confirmed. The arithmetic holds but for Nit 5. The reviewer read the
design and the code and ran nothing. Every finding was fixed at the next commit, Major 1 by reporting such a variant as
"not run under the durability rule" rather than relaxing or rewriting.

No Blocker.

## Major

1. (Plausible) The fallback for a vendor whose own code breaks a rule cannot be carried out: the hook refuses a
   vendor migration's `CREATE UNLOGGED TABLE` even when a logged table exists, creating it logged would change the
   vendor's source (Non-Goals), and a vendor that itself sets `synchronous_commit = off` has no configuration to fix.
   Fix: rewrite the persistence and count it, or report the variant as not run under the rule.

## Nit

1. Crash jobs have no rungs, so refusals before fault (c)'s kill are wiped uncounted; read the count with the insert
   LSN just before the kill.
2. Where the server image with the extension is built is not stated; build it in the history job, save it with its
   SHA-256 and name its exemption.
3. The extension's tests never run in CI; add them to the `competitive_driver` step.
4. The acknowledgement's wording predates the boundary snapshots and omits the `pg_class` scan.
5. The re-run rung's 0.5 minutes of overhead is missing: the job is 336.5 minutes, the campaign 18,765 runner minutes
   and 2,387 minutes of wall time.

Counts: Blocker 0, Major 1, Nit 5
