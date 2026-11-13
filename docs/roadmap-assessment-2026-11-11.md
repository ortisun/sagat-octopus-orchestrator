# Assessment: parity with Temporal and Windmill, and the path to better scale, performance and resilience

- **Date:** 2026-11-11
- **Reviewed commit:** `ed11d044` (tip of `develop`)
- **Requested by:** the owner
- **Status:** assessment for the owner. It decides nothing: it approves no feature, accepts no ADR and changes no
  scope. [roadmap.md](roadmap.md) and [owner-decisions.md](owner-decisions.md) stay the records of what is decided.

## How this was produced, and its limits

Six independent reads ran in parallel: the durable execution core, scalability and performance, resilience, the
Windmill-side product surface, the delivery ledger, and a web baseline of Temporal, Windmill and neighbouring engines.

- Nothing was compiled or run for the six reads. "Built" means the code and its tests exist, not that they were seen
  passing.
- Performance figures for this platform come from `tests/performance/reports/`, except the rerun of the transitions
  scenario recorded under "Rerun on current develop" below.
- Competitor figures are vendor claims unless marked otherwise.
- Confirmed a second time in code after the reads: the run-row lock and full step load per command
  (`crates/infra-postgres/src/execution.rs`, `RUN_FOR_UPDATE`), the refusal of a signal sent to a run that is not
  waiting (`crates/application/src/runs.rs`, "the run is not waiting for that signal"), the advisory lock on the
  claim path (`crates/infra-postgres/src/quotas.rs`, `lock_activity`), the absence of table partitioning in the
  migrations, the polled dispatch (`idle_retry` in `crates/api-grpc/src/service.rs`) and the benchmark figures.
- Reported by one reader and not confirmed a second time: the count of about 30 database round trips per
  transition, that no component subscribes to the broker hints, and the "not found" results for a general
  serialization-failure retry and a stuck-run detector.
- There is no file called "master prompt" in the repository. The original specification package (commit `140b03f8`),
  [constitution.md](constitution.md), `AGENTS.md` and `SPECIFICATION-INDEX.md` were read in that role.

## Verdict

The durable engine exists and is sound. What separates the product from "comparable to Temporal and Windmill" is
mostly product surface and horizontal scale. Today the platform is not better than either on scalability or
performance, and it is better on resilience only in part.

590 of 1,859 tasks are ticked. Features 019 to 032 and 036 (1,060 tasks) are drafts that the owner has not approved.

The original package contradicts the current goal on three points. It lists as non-goals "reimplementing Temporal or
Restate" and multi-region active-active, and `docs/architecture/overview.md` defers low-code. Drafts 018 to 032 need
those amended.

## What is built

| Area | State |
|---|---|
| Durable core | Append-only journal, one transaction for state, next steps, outbox, audit and idempotency, leases fenced by token, attempt and generation on the database clock, retries, timers, child workflows, a restricted continue-as-new |
| Sagas | Reverse-order compensation, reconciliation of uncertain effects, compensation breaker, operator review. Ahead of both products |
| Events | Webhooks (HMAC, OIDC, mTLS), inbound Kafka, NATS and SQS with dead letters and redrive |
| Scripts | Five languages in a container per run, signed artifacts, SBOM, mediated fetch and secrets |
| Clients | Six SDKs (the Rust one is in no tagged release yet), CLI with promotion between namespaces |
| Studio | Flow editor, Monaco script editor, run viewer with operator actions |
| Operations | Tenant quotas, scheduler fairness, 24 alert rules, fault suites with injected crash points, a DR drill that gates releases |
| In progress | 014 (37 of 45), 018 (19 of 81), 035 (20 of 129), 037 (32 of 34), 033 (6 of 7) |

## What is missing for parity

### Temporal side

| Gap | Covered by |
|---|---|
| Buffered signals (an early signal is refused today), signal-with-start, update-with-start, business ids with reuse and conflict policies | 018, partly built in the domain |
| Heartbeat details, completion by task token | 019 |
| Schedule-to-start and schedule-to-close timeouts, per-activity heartbeat timeout, non-retryable error types | no spec found |
| Search attributes, memo, list query, retention and archival | 020 |
| Worker deployments and versioning, named queues, priority, dispatch fairness, rate limits | 019, 022 |
| Time-skipping tests, replay check, interceptors | 028 |
| Client-side encryption, cross-namespace services, multi-region | 031, 030, 032 |
| Workflow as code. The model is a declarative graph interpreted by the engine, not replay of user code | no spec |

The last row is a product decision before it is an engineering one. The declarative model removes the
non-determinism and patching failure class and the replay cost; it also means no arbitrary control flow or local
variables in a workflow. Capability parity is reachable without it. Programming-model parity is not.

### Windmill side

| Gap | Covered by |
|---|---|
| Package dependencies for TypeScript and Python scripts (a script cannot import a library today) | no spec; gap G97 |
| Typed resources, variables, more triggers (custom HTTP routes, PostgreSQL CDC, MQTT, WebSocket, e-mail) | 021 |
| OAuth resource flows, an integration hub | no spec |
| Forms generated from schemas (starting a run is a JSON text area today), notifications | 026 |
| Step cache, per-key concurrency, debounce, early return, failure handler, single-step run | 022 |
| Loops in general use (`forEach` and `while` are refused on the default path) | 014, tasks T014-014 to T014-017 |
| App builder, folders, groups, custom roles | 027, 029 |
| Warm script containers (a script step costs roughly 0.45 to 0.7 s as recorded) | 019 US08 |
| More languages, data engines, AI agents and MCP | 023, 024, 025 |
| Two-way git sync, editor extension | 028 |

Studio also trails the engine it fronts. It does not show `switch`, `approval`, `forEach` or `while`, lists two
legacy node kinds that the server refuses, has no pages for schedules, the approvals inbox or the audit log, and
marks Go and Java scripts as not built although the backend accepts them. No single spec covers that catch-up.

## Scalability and performance

### Where it stands

- Best recorded figure: 1,513 transitions per second, closed loop, everything on one machine
  (`tests/performance/reports/scalability-ha.md`). 5,000 per second was not reached. The report attributes the
  ceiling to its harness.
- That section ran at commit `9092aa1` (2026-10-11), when there were 21 up-migrations. There are 87 now, and chain
  locks, collection and loop loads, durable-quota triggers and payload classification joined the hot path since.
  The figure is stale.
- One million suspended runs were measured, at about 2.8 KB each and a flat idle scheduler tick.

### Rerun on current develop (2026-11-11)

The transitions scenario alone was run again with `tests/performance/scalability-ha/run.sh` on the same machine
(Apple M3 Max, PostgreSQL 18.6, same server configuration), at `develop` `ed11d044` plus documentation commits. The
load average was 27 when it began (12 in October), so absolute figures are noisy; the wait profile is not.

| API replicas | Workers | October, transitions/s | Now, transitions/s | Now, share of active backends waiting on `Lock:advisory` |
|---:|---:|---:|---:|---:|
| 1 | 4 | 1,444 | 363 | 62% |
| 1 | 16 | 1,513 | 368 | 92% |
| 1 | 64 | 1,014 | 302 | 90% |
| 1 | 128 | 829 | 288 | 89% |
| 2 | 64 | 907 | 230 | 96% |
| 2 | 128 | 663 | 272 | 95% |
| 4 | 64 | 746 | 358 | 98% |
| 4 | 128 | 696 | 200 | 98% |
| 8 | 128 | 514 | 171 | 98% |

The best figure fell from 1,513 to 368 transitions per second, and 1,000 per second is no longer reached. In October
no backend waited on an advisory lock.

The lock was identified by its hash in `pg_locks` during a run, not by reading the code: it is
`quota:secrets:{tenant}:{namespace}`, taken exclusively by `current_worker_authorized`
(`crates/infra-postgres/src/execution.rs`) on every completion, failure and heartbeat of an ordinary worker and held
until commit. It was added on 2026-10-24 by `482979a6`. The check only reads the worker's membership; the lock keeps
the order of the catalog mutations. Held exclusively it makes the reports of one namespace wait for each other.

A first attribution, made from the dates and the code alone, blamed the per-activity admission lock of
`activity_admitted` (`ee2063a0`, 2026-10-12). Removing that mutex for an unbounded limit left the benchmark where it
was (369 transitions per second at best, 62 to 99% still on `Lock:advisory`), so it is a real mutex and not this
bottleneck. Whether sharing the secrets lock restores the October figure is not measured yet: the run that would show
it ended with the machine's disk full.

### Numbers to beat

| Reference | Figure | Kind |
|---|---|---|
| Temporal self-hosted walk-through | 1,350 state transitions/s on a 4-core database | vendor |
| Temporal Cloud | over 150,000 actions/s; start p99 78 ms, signal p99 91 ms | vendor |
| Hatchet on PostgreSQL | 2,000 runs/s on 8 vCPU | vendor |
| DBOS on PostgreSQL | 12,100 queued and 43,000 direct workflows/s on 96 vCPU | vendor, public benchmark code |
| Restate | 16,844 three-step workflows/s, p99 98 ms, three-way replication | vendor |
| Windmill dedicated workers | up to 1,000 steps/s per worker; 54 ms overhead per job | vendor |

Nobody publishes crash-recovery times. Measuring and publishing them is an opening.

### Why it is slow today

1. Every command locks the run row and loads every step, saga, collection, loop and chain of the run. Cost grows
   with run size, and a wide run is one hot row. A heartbeat is a full run transaction.
2. The claim reads candidates without `SKIP LOCKED`, so pollers of one activity read the same rows, then pays one
   transaction per candidate run.
3. Each lease takes an exclusive advisory lock per tenant, namespace and activity and counts in-flight steps.
4. Dispatch is polled: an empty stream sleeps 500 ms, the scheduler ticks every 500 ms. Broker hints are published
   but, as reported, nothing subscribes to them.
5. `steps` carries many partial indexes keyed on status, and row triggers add a quota delta row per write.
6. One PostgreSQL primary is the ceiling. Only scheduler work is sharded (64 leases). No table is partitioned, and
   `runs`, `steps`, `run_events` and `payloads` are never deleted.

### Remedies

| Remedy | Covered by |
|---|---|
| Load a run in one statement, batch journal inserts, skip loads the definition does not need | no spec |
| Claim simple root steps with one `UPDATE ... FOR UPDATE SKIP LOCKED` over a lean queue table | no spec |
| Serialise the leases of an activity only when its effective limit is bounded; a default (`MAX_LIMIT`) limit needs a fence against a policy change, not a mutex or a count | no spec |
| Share, not hold, the namespace's secrets lock in the worker authorization check of every report | no spec |
| Wake parked streams on commit; return the next ready step in the completion response | no spec |
| Partition `run_events` and `payloads`; retention and archival | 020 covers retention, not partitioning |
| A statement-count and throughput budget as a CI regression gate | no spec |
| Cells: a database per group of tenants behind a router | 032 defines cells for failover only |

With the first four, the performance reader estimates 5,000 to 15,000 transitions per second on one tuned primary.
That is an estimate, not evidence. Past that, only cells raise the ceiling.

## Resilience

### Stronger than both today

- The broker is disposable: workers claim through gRPC against PostgreSQL, and losing NATS or Redis costs nothing.
- Leases use the database clock and a generation fence, which removes the clock-skew failure class.
- Work with an unknown outcome is held for reconciliation or review, not re-run blindly.
- One stateful dependency and one consistent backup, with a replay-equals-projection check after restore.

### Gaps, by blast radius

| Gap | Covered by |
|---|---|
| PostgreSQL high availability is left to the operator; promotion and the address switch are manual | new work |
| Upgrades need a stop-the-world window; mixed old and new writers are unsupported | no spec |
| Journal and audit grow without bound, which degrades vacuum, backup size and restore time | 020 |
| DR evidence is loopback on one machine and leaves out the object store, the key ring and remote backups | G250, G251, no tasks |
| No region survival | 032 |
| No breaker or flow control on forward activities and connectors | 022 |
| Node drain never tested; zone spread is `ScheduleAnyway`; the sandbox runner has no chart template | G247 |
| No measured SLOs, no "run made no progress" detector, no dual control on skip-compensation | 008 proposes SLOs |

## Recommended order

1. **Measure first.** Rerun the harness on current `develop`, then on separate machines with open-loop load, and
   run the same workflow shape on Temporal and Windmill. Without it every later claim is a guess.
2. **Hot path.** The first four remedies above, under a new spec, with the regression budget in CI.
3. **Close 014 and 018**, and add journal partitioning with retention (020).
4. **Operational resilience.** A reference HA topology, expand and contract migrations with an N and N-1 upgrade
   test, a three-zone disruption and failover drill.
5. **Temporal parity.** 019, 020, 022, and a spec for the missing activity timeouts.
6. **Windmill adoption.** Script dependencies, 021, 026, and the Studio catch-up.
7. **Later.** Cells and 032, then 023 to 025 and 027 to 031.

## Decisions that are the owner's

- Approve, cut or reorder drafts 019 to 032 and 036, and accept or reject the 25 proposed ADRs.
- Decide whether "comparable to Temporal" includes workflow as code.
- Amend the constitution on single region (O1560), on the non-goal of reimplementing Temporal, and on low-code
  (O1360).
- Decide O1081, the rule for `pii` leaving the platform, which gates 020, 025, 026 and 031.
- Features 013 (federated login) and 015 are listed in the roadmap and have no specification in the repository.

## Document drift noticed on the way

- The roadmap row and the spec header of 035 say nothing is built; 20 tasks are ticked and `crates/infra-lake`
  exists.
- The spec headers of 001 to 012 still say "Draft — Ready for review" although they shipped in 0.2.0.
- `SPECIFICATION-INDEX.md` and the roadmap's parity paragraph say none of 018 to 032 is approved; 018 is.
- `AGENTS.md` and `CLAUDE.md` still point to `.specify/`, which no longer exists.
- `tasks/plan.md` and `tasks/todo.md` stop at 2026-10-13.
- `docs/traceability.md` has no requirement tables for 018, 033, 034 or 035.
