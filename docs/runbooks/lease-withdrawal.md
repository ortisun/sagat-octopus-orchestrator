# Lease withdrawal (feature 006, ADR-0098)

A lease the execution engine committed and never handed to a host used to lapse after its time-to-live; since feature 037
a lapsed lease of an `Activity` opens a review (`step.needs_review`) for work that never ran. With the requirement
`lease-withdrawal-v1` recorded, the engine withdraws such a lease instead: the step goes back to `READY`, the attempt is
not spent, and the journal records `step.lease_withdrawn` (cause `NOT_SENT`). The decisions are D006-001 to D006-018 and
D006-030 to D006-039 and D006-080 to D006-091 of `specs/006-sdk-multilanguage/research.md`.

## What changes, and what does not

- Nothing changes until the operator records the requirement. Before that, the counter
  `worker_leases_undelivered_total{cause, outcome="lapsed"}` shows what activation would recover.
- A host of protocol 1.8 (delivery acknowledgement) or later that drops a lease it received hands it back with `FailStep`
  `TASK_NOT_STARTED`, and the engine withdraws it (cause `NOT_STARTED`). A host below 1.8 does not hand back: the lease
  lapses as before (decision D006-080). This build negotiates at most 1.6, so today no host hands back. A raw client or an
  older SDK that sends the code below 1.8 gets an ordinary failure: the attempt is spent and retried with its backoff,
  never reviewed (gap G006-030).
- One attempt is withdrawn at most four times (decision D006-031). The fifth is refused and counted `capped`: a hand-back
  becomes an ordinary failure, and a lease the engine did not send lapses as before.
- An engine runs at most a quarter of its pool of withdrawals at once (`max(1, SAGAT_DB_MAX_CONNECTIONS / 4)`, 5 with the
  default pool of 20; decision D006-038). The others wait without holding a connection.
- A frame lost on the wire after it left the process, and a lost answer of a unary poll, still lapse into a review
  (gap G006-001). A process that crashes between a claim and its withdrawal leaves the lease to lapse (gap G006-002). A
  graceful shutdown waits for the claims in progress and their withdrawals, within the shutdown's grace.
- A lease that lapses keeps its meaning: an unknown outcome, consultation or review (feature 037, FR-037-006).

## Before upgrading

On a large journal, build the partial index of `20270208000200_lease_withdrawal_floor` before you migrate, so the
migration does not hold journal writes while it reads `run_events` (decision D006-085). With the owner role:

```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS run_events_lease_withdrawn_idx ON run_events (tenant_id)
    WHERE event_type = 'step.lease_withdrawn';
```

A build that fails leaves an invalid index, which the migration refuses: drop it (`DROP INDEX CONCURRENTLY
run_events_lease_withdrawn_idx;`) and build it again.

## Before the activation

1. Every control plane and engine runs this build or a later one. A process of this build lists `lease-withdrawal-v1`
   unconditionally. **A binary of release 0.2 does not register in the instance-capability registry, so an activation cannot
   see it** (`docs/runbooks/instance-registry.md`, section 7): stop or upgrade every 0.2 process first. A 0.2 binary cannot
   replay `step.lease_withdrawn`.
2. Count the live processes you expect (control planes and engines together).
3. Know that the activation is one-way once a withdrawal happens (see Rollback).

## Activate

Run with the schema-owning database role (the `DATABASE_URL` of `migrate`):

```sh
control-plane capability-activate lease-withdrawal-v1 --operator NAME --expect-live N --dry-run
control-plane capability-activate lease-withdrawal-v1 --operator NAME --expect-live N
```

`capability-activate` is the one command for every fleet requirement an operator activates (decision D006-034); feature
022 uses it for its own capabilities.

- `--expect-live N`: the command refuses when the live count (seen within 30 seconds) differs; without it, when none is
  live. The count is taken again under the epoch lock.
- An entry may also require at least one live process of each kind it covers (`each_kind_live`, decision D006-082);
  `lease-withdrawal-v1` does not, and may be activated with nothing live (`--expect-live 0`).
- A live process that does not list the capability is refused by its instance id; nothing is recorded.
- A second run says the requirement is recorded already and exits 0.
- Each engine reads the requirement at its next heartbeat (within about ten seconds) and withdraws from then on.
- The requirement counts only when it is recorded for control planes and engines, as the command records it. A row
  recorded by hand SQL for one kind withdraws nothing: every withdrawal reads the row again under its lock and finds it
  incomplete (decision D006-095).

## Activate `lease-withdrawal-v2`

`lease-withdrawal-v2` is the floor's own capability (decision D006-096; ADR-0099): a process that lists it replays every
cause of `step.lease_withdrawn`, the cause `NOT_ACKNOWLEDGED` of protocol 1.8 included, and, for an engine, serves no
capability call while its registry has been silent (`docs/runbooks/instance-registry.md`, section 3). Recording it keeps
every older binary out of the fleet, a build of `lease-withdrawal-v1` alone among them. It needs `lease-withdrawal-v1`
recorded for both kinds first; the command refuses otherwise, and checks again under the epoch lock.

```sh
control-plane capability-activate lease-withdrawal-v2 --operator NAME --expect-live N --dry-run
control-plane capability-activate lease-withdrawal-v2 --operator NAME --expect-live N
```

The other rules are those of `lease-withdrawal-v1` above. On a large journal, build its partial index before you migrate,
as for `run_events_lease_withdrawn_idx`:

```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS run_events_lease_not_acknowledged_idx ON run_events (tenant_id)
    WHERE event_type = 'step.lease_withdrawn' AND detail->>'cause' = 'NOT_ACKNOWLEDGED';
```

It can be removed again (`platform_requirement_remove('lease-withdrawal-v2', ...)`, under READ COMMITTED and the 1.5 s
statement timeout, as below) until a journal holds a withdrawal with the cause `NOT_ACKNOWLEDGED`. From then on the removal
is refused (`lease-withdrawal-v2 cannot be removed while a journal holds step.lease_withdrawn with the cause
NOT_ACKNOWLEDGED`), and the down migrations `20270211000150_lease_withdrawal_v2_floor` and `20270211000100_lease_withdrawal_v2`
refuse as well. The migration that builds the index (`20270211000150`) touches no registry table, so heartbeats never wait
for its read of the journal. This build writes no such
withdrawal: only a build of protocol 1.8 does.

### The forward order to protocol 1.8

1. Every control plane and engine runs the build of the follow-up of pull request #260 (it lists `lease-withdrawal-v2`;
   check `serving_instances`, section 5 of `docs/runbooks/instance-registry.md`).
2. `lease-withdrawal-v1` is activated, then `lease-withdrawal-v2` (above).
3. The build of protocol 1.8 is deployed.
4. `delivery-acknowledgement-v1` is activated (its own runbook, with that build).

A rollback walks the same steps back; once a journal holds a `NOT_ACKNOWLEDGED` withdrawal, the binaries roll back no
further than step 1's build (ADR-0099, decision 10; decision D006-059 request (9)).

## Watching it

| Signal | Meaning |
| --- | --- |
| `worker_leases_undelivered_total{outcome="withdrawn"}` | Leases given back; the steps are `READY` with their attempt unspent |
| `worker_leases_undelivered_total{outcome="fence_missed"}` | The step had moved on (a heartbeat started it, it was reported, the run ended, the reaper ended the lease); nothing was changed |
| `worker_leases_undelivered_total{outcome="lapsed"}` | The requirement is not recorded (or not yet read by this engine): the lease lapses, and a hand-back of a 1.8 host is answered `STALE_ATTEMPT` and lapses too (decision D006-089); or the host's session is older than 1.8: its hand-back is an ordinary failure |
| `worker_leases_undelivered_total{outcome="capped"}` | The attempt was withdrawn four times already: a hand-back is an ordinary failure, a lease not sent lapses |
| `worker_leases_undelivered_total{outcome="disabled"}` | This engine runs with `SAGAT_LEASE_WITHDRAWAL=off`: a lease not sent lapses, and so does a lease a 1.8 host hands back (answered `STALE_ATTEMPT`, decision D006-089) |
| `worker_leases_undelivered_total{outcome="error"}` | The store failed or the process was stopping past its grace; the lease lapses |
| `step.lease_withdrawn` in a run's timeline | One withdrawal, with its cause (`NOT_SENT`, `NOT_STARTED`) |

A growing `capped` count means a loop: a client that keeps dropping its calls, or a host that keeps handing the same task
back. Find the steps with the query below and look at the hosts of their activity.

```sql
SELECT tenant_id, run_id, node, instance_path_indices, attempt, count(*)
  FROM run_events WHERE event_type = 'step.lease_withdrawn'
 GROUP BY 1, 2, 3, 4, 5 HAVING count(*) >= 3 ORDER BY 6 DESC LIMIT 50;
```

## Stopping it

Set `SAGAT_LEASE_WITHDRAWAL=off` on every execution engine (a mixed fleet withdraws on some replicas and not on others) and restart them. They keep listing `lease-withdrawal-v1`
(they can still replay the event) and stop writing new withdrawals; their leases lapse as before, counted `disabled`. Set
it back to `on` (the default) to resume. Do not remove the requirement to stop: see Rollback.

## Rollback

The requirement is a replay floor (decision D006-032). A build older than this feature cannot replay
`step.lease_withdrawn`, so once one journal holds the event:

- `platform_requirement_remove('lease-withdrawal-v1', ...)` is refused (`lease-withdrawal-v1 cannot be removed while a
  journal holds step.lease_withdrawn`; the message ends by saying that `SAGAT_LEASE_WITHDRAWAL=off` stops new withdrawals
  of unmarked leases, which is all the switch does from protocol 1.8). The check is a probe of the partial index
  `run_events_lease_withdrawn_idx`.
- The removal runs only under READ COMMITTED, the default (decision D006-084); a session or role whose
  `default_transaction_isolation` is stricter is refused (`is removed only under READ COMMITTED`): run it with
  `SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL READ COMMITTED` first.
- The removal waits at most one second for each lock (decision D006-085); run it under `SET statement_timeout =
  '1500ms';`, which bounds the whole call, so registrations, which queue behind it on a 5 s statement timeout, are never
  held past it. An engine still withdrawing holds the lock it waits for: a removal that fails with `canceling statement
  due to lock timeout` (or `statement timeout`) changed nothing. Set `SAGAT_LEASE_WITHDRAWAL=off` first and run it again.
- The down migration `20270208000000_lease_withdrawal` refuses as well (`20270208000200_lease_withdrawal_floor` rolls back
  first and holds nothing an older binary cannot read).
- A binary rollback below this feature is not possible. Roll back to a build of this feature or later.

While no journal holds the event (`SELECT count(*) FROM run_events WHERE event_type = 'step.lease_withdrawn'` is 0), the
way back is: set `SAGAT_LEASE_WITHDRAWAL=off` on every engine and wait one heartbeat interval (about ten seconds), remove
the requirement with the owner role (`SET statement_timeout = '1500ms'; SELECT * FROM platform_requirement_remove('lease-withdrawal-v1', 'your-subject');`),
then roll the binaries back. The down migration also refuses while the requirement or its history exists, or while a
registered process lists the capability.
