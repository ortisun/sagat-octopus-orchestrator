# Targeted re-read 2 of design 051: correctness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 11b5f3093c972e1da022d17655640bff7bcfc6ba
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Run as the approval
confirmation; it found Blockers introduced by the security fixes, so it is filed as a re-read. All twelve findings of
`reread-1-correctness.md` are resolved (Nit 3 by one reset per turn with snapshot differences), and the arithmetic
rechecked holds (turn 103, throughput job 336, common-rate job 92, crash job 185, timer-burst job 128, history job 230,
campaign 18,745 runner minutes, wall time 2,385 minutes). The reviewer read the design, the reports, this platform's
PostgreSQL code and migrations and the sqlx-postgres 0.9.1 source, and ran nothing. Every finding was fixed at the next
commit.

## Blocker

1. `pg_proc.proconfig` must be empty on every function, but this platform's migrations set `search_path` on dozens of
   functions (for example `20261024000000_durable_quotas.up.sql:79`), so every turn of this platform fails. Fix: a
   per-product list of settings unrelated to durability, with a fixture from this platform's migrated schema.
2. This platform's pool always sends `options` with `-c statement_timeout` and `-c idle_in_transaction_session_timeout`
   (`crates/infra-postgres/src/pool.rs:112-124`) and SQLx sends `extra_float_digits`, both refused; and
   `pg_stat_activity` shows only `application_name`. Fix: parse `options` and admit `-c` only for listed settings
   unrelated to durability, allow `extra_float_digits`, and apply the allowlist to the published options.
3. Fault (c)'s kill empties `pg_stat_statements` and gives it a new `stats_reset`, so every fault (c) turn fails the end
   scan. Fix: scan just before the kill, admit the restart's reset time with `dealloc` at 0 from then on, and state the
   gap as a residual.

## Major

1. A read bounded at 1 MB of the whole execution file sees only finished activities after a few minutes, so fault
   (b)'s in-flight trigger always waits the 30 s limit. Fix: keep the descriptor and read on from the last offset,
   bounding each poll.
2. (Plausible) No channel between the driver in its container and the host side, which must act at each rung's
   boundaries and learn the baseline's and null fault's ends. Fix: a directory only the driver writes, read by the host
   under the same rule, and an acknowledgement in the host-only directory before each rung.
3. With `track = all`, nested statements are counted inside their caller's time too, and keying on `queryid` alone
   collides top-level and nested rows. Fix: key on (`userid`, `dbid`, `queryid`, `toplevel`) and publish top-level and
   nested time apart.
4. (Plausible) Shims that exit during the turn leave `/proc`, so this platform's script variants are under-charged.
   Fix: the difference of `usage_usec` in `cpu.stat` of the `docker.service` and `containerd.service` control groups.

## Nit

1. Task 051-7's test still compares on the driver's monotonic clock while the injector writes `CLOCK_REALTIME`, and
   "in `run.sh`'s process" cannot call `openat2` or `pidfd`; name the host binary.
2. The snapshot instants are vague (the 300 s window or the whole rung), and Task 051-6 omits the `stats_reset` check.
3. `pg_stat_statements.max` is not among the fixed constraints while a `dealloc` above 0 fails the turn.

Counts: Blocker 3, Major 4, Nit 3
