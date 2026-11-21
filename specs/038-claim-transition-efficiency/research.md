# Research: claim and transition efficiency (design 038)

Measurements and what they attribute. Append-only: a later measurement is a new dated section.

## 2026-11-12 — What changed between October and now (T038-2, first half)

**Method.** The transitions scenario of `tests/performance/scalability-ha/main.rs`
(`step_transitions_per_second`, `SAGAT_PERF_TRANSITION_RUNS=1000`: nine configurations, 27,000 leases and
27,000 completions) was run twice on one PostgreSQL 18.6 server with `pg_stat_statements` on, a few
minutes apart, at a load average of about 11: once from a checkout of `9092aa1` (the commit of the
October report) and once from the stack of pull requests 203, 205, 206, 207 and 208. The server ran with
`fsync=off`, so the rates below are comparable with each other and with nothing else; the report's own
figures are taken with `fsync=on`. Only top-level statements are counted.

**Rates, same server, same day.**

| API replicas | Workers | `9092aa1` | The stack |
|---:|---:|---:|---:|
| 1 | 4 | 1,891 | 786 |
| 1 | 16 | 1,354 | 838 |
| 1 | 64 | 648 | 338 |
| 1 | 128 | 447 | 226 |
| 2 | 64 | 684 | 360 |
| 2 | 128 | 502 | 284 |
| 4 | 64 | 813 | 369 |
| 4 | 128 | 613 | 297 |
| 8 | 128 | 516 | 271 |

The October code is 2.3 times faster at its best on the same machine at the same time. The regression
is in the code, not in the machine or its load.

**Counts.**

| What | `9092aa1` | The stack |
|---|---:|---:|
| Statements, with `BEGIN`, `COMMIT` and `ROLLBACK` | about 4.58 million | about 9.42 million |
| per transition | about 170 | about 349 |
| Polls (candidate scans) | 14,899 | 26,099 |
| Candidates returned | 891,976 | 1,574,229 |
| Candidates probed | 886,015 | 1,572,424 |
| Statements of one probe that gives up | 3 (`BEGIN`, the run row, `ROLLBACK`) | 4 (the chain lookup added) |
| Runs loaded by a claim | about 194,000 that leased nothing, and 27,000 that did | 28,718 in all |
| Runs loaded per lease | about 8.2 | 1.06 |

**What this attributes.**

1. **Throughput follows round trips.** Statements per transition doubled (170 to 349) and the best rate
   fell 2.3 times. With the lock waits gone (pull request 203) the backends are `CPU:running` and
   `Client:ClientRead`: they execute a statement and wait for the next.
2. **The herd is not the regression.** October probed 33 candidates per lease too. It was 58% of the
   round trips then (886,015 probes of 3 statements) and is 67% now (1,572,424 of 4). It was always the
   largest single cost; the report's fall from 1,513 to 514 with replicas was this.
3. **A command costs about three times the statements it did.** Without concurrency a lease is 42
   statements and a completion 54 on the stack (`work_budget.rs`). The October profile leaves about
   0.88 million statements for its 54,000 commands and 9,000 starts once probes and failed claims are
   taken out, about 14 per command. That is an estimate from totals, not a measurement of one command at
   that commit. What a command runs now that it did not: the chain lookup and the chain row; the loads
   of scope sagas, scopes, collections, loops and declared state; three reads and a write of the output
   byte usage; three statements of the deferred quota flush for every statement that writes `steps` or
   `runs`; the clock after each lock; the activity fence and limit; the membership and binding checks
   twice in a completion. This matches the completion's median, 3 ms where it was 1 ms.
4. **The drop of a stale candidate works.** Runs loaded per lease fell from 7.5 (pull request 203 alone)
   to 1.06, below SC-038-001's 1.5.
5. **Probes per lease rose, from 33.5 to 58.** The scenario is a closed loop with more demand than
   supply: workers poll as fast as they can, a poll walks up to 64 candidates, and a probe is now cheap,
   so they spin faster. SC-038-001's bound on probes (4 per lease) is not met and cannot be without
   changing which candidates a poll tries (T038-8) or how far it walks.

**What follows for the order of work.** By measured weight: the probes (two thirds of all round trips),
then the statements of a command. T038-8 and a bound on how far a poll walks address the first; T038-9
and folding the projection loads address the second. Neither the membership row nor the session row
shows as a wait any more (`Lock:transactionid` 1% with eight replicas on the stack), so the follow-up
design the first draft expected for them has no measurement behind it now.

**Not done.** The second half of T038-2, naming the contended rows with eight replicas, is moot while
the wait is at 1%. The estimate of 14 statements per command in October was not confirmed by running the
budget test at that commit: its test support differs.
