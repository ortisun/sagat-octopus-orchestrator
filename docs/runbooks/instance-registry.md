# Runbook: the instance-capability registry (feature 018, shared with 029 and 031)

For operators who upgrade or roll back the control plane and the execution engine, and who turn on a feature that every one of
their processes has to honour. It says what the registry is, what a process does when it starts and while it runs, what you see
when one refuses, how to read the tables, what the registry does not protect against, and what to do when a requirement blocks
the fleet.

Design: [ADR-0082](../adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md),
decisions D1034 and D2013 to D2032, gaps G1011, G1607 and G1996 to G2003 in
[`specs/018-schedules-bulk-operations/research.md`](../../specs/018-schedules-bulk-operations/research.md); the tables are in the
"Instance-capability registry" section of that feature's
[`data-model.md`](../../specs/018-schedules-bulk-operations/data-model.md); the rule is ADR-0050 decision 9, and
[ADR-0081](../adr/ADR-0081-cancel-source-is-switched-on-per-process-until-the-capability-registry-exists.md) says what the
per-process flags of the other features still do until they move to it.

## 1. What it is

A table of the **serving** processes, those that hold platform database credentials and serve: each control-plane replica and
each execution-engine replica. The one-shot commands (`migrate`, `provision-tenant`, `cleanup-secrets`) do not register. Runners
and SDK workers hold no database credentials and are not in it (they keep the worker-protocol negotiation of ADR-0015). Each
process writes one row about itself when it starts, says it is alive every 10 seconds, and deletes the row when it has stopped
cleanly. It uses one database connection of its own for this, besides the pool of its requests.

| Table | Holds | Who writes it |
|---|---|---|
| `serving_instances` | one row per process start: `instance_id` (random), `kind` (`CONTROL_PLANE`, `EXECUTION_ENGINE`), `binary_version`, `capabilities`, `scheduler_owner` (an engine's name in `scheduler_members`, for correlation), `started_at`, `last_seen`, and the hash of a token only that process holds (the runtime role cannot read it) | the process itself, through three functions (`instance_register`, `instance_heartbeat`, `instance_deregister`), each of which touches a row only for the token that registered it |
| `platform_requirements` | what the platform requires: a `capability` and the `kinds` that must hold it, who recorded it and since when | only the operator functions |
| `platform_requirement_epoch` | one number, moved by one in every transaction that records or removes a requirement | only the operator functions |
| `platform_requirement_history` | every activation and removal: who the command said, which database role did it, what, which epoch, when (append-only; the runtime role cannot read it) | only the operator functions |

The capabilities are a closed list: `schedules-v1` (feature 018, the engine), `access-narrowing-v1` (029, the control plane) and
`payload-encoding-v1` (031, both). **A process lists a capability when the process will honour it**: what its binary can do,
narrowed by the passes its configuration turns on. An engine lists `schedules-v1` only when it runs the schedule pass
(`SAGAT_SCHEDULES_ENABLED`); an engine started without it lists nothing, because after the handover of the cron triggers it
would fire neither.

A process is **live** when it was seen in the last 30 seconds, on the database's clock (so replicas with skewed clocks agree).
A row silent for more than a day is deleted by any heartbeat. The table holds at most 1,000 rows, and when it is full the rows
that are no longer live give way to a new registration, oldest first.

`scheduler_members` is not touched: its rows are the shard leases, and the keeper divides the 64 shards by the replicas it lists.

## 2. A process is refused

At start, after it has its database pool and has checked its role, and **before** it answers its first request (control plane) or
runs its first pass (engine), a process asks the registry, in one transaction, whether the platform requires something of its kind
that it cannot do. If it is refused, or the registry cannot be asked, **it does not exit**: it stays alive, serves nothing, says
why, and asks again every second, so that it serves as soon as the registry lets it, with no restart. That is how a process
started before its migration has always behaved (alive, not ready, ready once migrated). "Serves nothing" means: every call but
the liveness probe is `503`; readiness fails; an engine runs no pass, reads no connector, holds no shard and answers its workers
`UNAVAILABLE`.

| What you see | Meaning | What to do |
|---|---|---|
| Log line `BINARY_CAPABILITY_MISSING: the platform requires <capability> and this process lacks it; ...` (`reason="BINARY_CAPABILITY_MISSING"`), once; control plane: every call but `/v1/health` is `503` with `details.reason` `BINARY_CAPABILITY_MISSING` and `/v1/ready` fails; engine: `/readyz` is `503` with the same reason, `/livez` is `200`. Alert `SagatInstanceGateClosed` after five minutes | A requirement is recorded for this kind and this process does not list it: its build is older than the feature that was activated, **or its configuration does not run the pass that provides it** (an engine restarted without `SAGAT_SCHEDULES_ENABLED` after the handover) | Deploy a build that has the capability, or set the flag that runs the pass, or stop the process. Do **not** remove the requirement to make a process serve unless you mean to roll the feature back (section 5): when the requirement goes, the process serves by itself |
| Log lines `REGISTRY_UNREACHABLE: ...` (`reason="REGISTRY_UNREACHABLE"`), then `could not say this instance is alive` every second; `/v1/ready` and `/readyz` are `503` with no reason in the answer | The registry could not be asked: the database is down, or the migration `20270113000000_instance_registry` has not been applied | Run `control-plane migrate` with the owner role. Nothing needs restarting: the process registers at its next try. A process never serves without the registry |
| Log line `REGISTRY_FULL: ...` at start (`reason="REGISTRY_FULL"`) while the process **serves** | 1,000 processes were seen in the last 30 seconds. The check of capabilities passed, so the process serves, and an activation cannot see it | Something is starting processes in a loop, or the fleet is far larger than designed, or something wrote rows of its own (section 6). Rows that are no longer live give way to new ones, so this means 1,000 **live** rows: stop the ones that should not run |

A process that is refused leaves **no row**, so a crash loop does not fill the table. (Rows of processes that died without
leaving stay for a day for you to read, but they give way to a new registration when the table is full.) The orchestrator keeps
it out of rotation because it is not ready; it does not restart it because it is alive.

## 3. While it runs

Every 10 seconds (±20 %) each process updates its own `last_seen` and learns the requirement epoch. When the epoch has moved it
reads the requirements again, and if one names a capability it lacks, **it stops serving**:

- **Control plane:** every call but `GET /v1/health` is answered `503 UNAVAILABLE` with `details.reason =
  BINARY_CAPABILITY_MISSING` and a `Retry-After`; `GET /v1/ready` fails with the same reason, so the load balancer sends traffic
  to the replicas that can serve. Streams end with a `reconnect` event. `GET /v1/health` stays `200`: a restart would only be
  refused again.
- **Engine:** `/readyz` answers `503` with the same reason in the error envelope; `/livez` stays `200`. Every loop pauses (the
  scheduler, the schedule, batch and reset passes, the event dispatcher, the outbox dispatcher, the deliveries, the telemetry
  projector, the monitor, the housekeeping and the sweep of blobs); the supervisor of the connectors winds down every consumer and
  starts none (no event is admitted from a broker); the keeper gives its shards back and leaves the replicas that count, so the
  others take the work; the worker API answers `UNAVAILABLE` with `x-sagat-error: BINARY_CAPABILITY_MISSING` before it
  authenticates, and a task stream that was open leases nothing more (it ends with that answer at its next claim). A task stream
  is not cut in the middle of a frame.

When the requirement is removed, the next beat opens the gate again and the engine takes its shards again.

**How long it takes.** A process learns of a requirement at its next beat: at most about 12 seconds after the activation
committed. Only a process that was **not live** at the activation can be in this state, because an activation is refused while a
live process of the required kinds lacks the capability.

**If the registry cannot be reached while the process serves**, the beat fails, a warning with the instance id is logged, and it
is tried again at the next beat. After 30 seconds the line becomes an error and **what happens next depends on the failure**:

- A **lost connection** to the database (the connection fails, the pool cannot give one, the server shuts down): the process
  keeps serving. It needs the database for everything it does, so a registry that cannot be reached is a database that cannot be
  reached, and stopping the fleet for it would make a short outage longer. From 30 seconds on an activation no longer counts it,
  so an activation may record a requirement it lacks; its first successful beat closes its gate.
- **Any other failure** (a lock or statement timeout on its own row, a permission or function error): the gate is closed. The
  process would otherwise be invisible to activations and would never learn of an epoch change while the database works. The next
  heartbeat that succeeds reads the requirements and decides again. **This is a precondition for activating 029's or 031's
  capability:** the fleet must run builds that have this rule.

**Readiness** also fails when the registry has not answered for more than 30 seconds, whatever the gate says (alert
`SagatInstanceRegistrySilent` after a minute more). If the heartbeat itself ends (it panicked, it was aborted), the process stops:
the control plane exits with an error, the engine fails as it does for any critical task.

A clean stop deletes the row **after** the server has drained: an activation in the grace window still counts a replica that is
still finishing its work.

## 4. Metrics and alerts

| Metric | Type | Useful for |
|---|---|---|
| `serving_instances_live{kind}` | gauge | The processes seen in the last 30 seconds, per kind. Every process sets the same value (the fleet's), so read it with `max`, never `sum`. A kind with none is `0`. Compare with the replicas you meant to run |
| `instance_registry_refusals_total{reason}` | counter | A process refused to serve or to be counted (`BINARY_CAPABILITY_MISSING`, `REGISTRY_FULL`, `REGISTRY_UNREACHABLE`), at start or when it learned of a requirement it lacks or failed closed. One-shot per event: use `SagatInstanceGateClosed` to see a process that stays refused. No instance id is a label |
| `instance_registry_gate_open` | gauge | `1` while this process serves, `0` while its gate is closed. Per process |
| `instance_registry_last_beat_age_seconds` | gauge | Seconds since the registry last answered this process; it grows while the registry does not answer. Per process |

Alerts (`infra/observability/alerts.yaml`): `SagatInstanceGateClosed` (a gate closed for five minutes, runbook section 2) and
`SagatInstanceRegistrySilent` (the last answer older than 30 seconds for a minute, section 3). The line in the log of the refused
process carries the instance id (`instance`), the epoch and the capability names.

## 5. Reading the tables, and recording or removing a requirement

Read-only, as the owner or `sagat_app` (the hash of the token and the history need the owner):

```sql
-- who is live, per kind, with what
SELECT kind, binary_version, capabilities, scheduler_owner, last_seen > clock_timestamp() - interval '30 seconds' AS live
  FROM serving_instances ORDER BY kind, last_seen DESC;
-- what is required, and the epoch
SELECT capability, kinds, required_since, recorded_by FROM platform_requirements;
SELECT epoch FROM platform_requirement_epoch;
-- who recorded or removed what, and as which database role (owner only)
SELECT id, action, capability, kinds, actor, database_user, epoch, at FROM platform_requirement_history ORDER BY id;
```

**Recording a requirement is an operator act with the owner role**, never a tenant's or the API's: the functions that do it
(`platform_requirement_lock`, `platform_requirement_activate`, `platform_requirement_remove`) are not executable by the runtime
role, so a compromised API process cannot make the fleet refuse to serve by recording one (section 6 says what it still can do). A command that runs as `sagat_app`
fails with `permission denied for function`: run it with the owner role, as `migrate` is run. Each feature that activates through
the registry has its own command (`control-plane schedule-trigger-handover` for feature 018, a later task), which in **one
transaction** locks the epoch, verifies every live process of the required kinds, records the requirement, moves the epoch,
writes the history and does the feature's own step. The transaction is bounded (lock wait 2 s, statement 3 s, the feature's step
3 s): a step that takes longer is rolled back, because every process that registers waits for the epoch meanwhile. **The
feature's step reads, it does not lock**: it must not take a lock that a transaction reading the epoch may hold. (The one
step that locks, because a publication in flight is invisible to a count, is the cron-trigger handover: it takes the table of
the cron triggers for at most two seconds and the handover row, which no transaction that reads the epoch ever takes; ADR-0083
narrows this rule for it and says why.) If a live
process lacks the capability the command is refused and says how many and which (up to sixteen instance ids): upgrade or stop
them, wait 30 seconds if you stopped them without a clean stop, and run it again.

**Before activating**, check `serving_instances_live` and the query above: an activation with no live process of the kind passes
vacuously, and a process that starts afterwards without the capability is then refused. (The command of a feature is to refuse
when fewer processes are live than the count the operator names, G2002.)

**A requirement that blocks the fleet** (for example every engine is refused because the feature was activated before all of them
were upgraded, and the new build has to be rolled back). Removing a requirement is the activation in reverse, and the feature
must first confirm that nothing depends on it. Only when the feature's own removal command does not exist yet, as the owner:

```sql
BEGIN;
SELECT platform_requirement_lock();                                    -- the epoch, FOR UPDATE
-- (the feature's own check that nothing depends on the requirement goes here)
SELECT * FROM platform_requirement_remove('schedules-v1', 'your-subject');
COMMIT;
```

The processes that were refused start serving at their next try (within a second). The history keeps both acts. A hand edit of
`platform_requirements` or of the epoch is refused by triggers that hold even for the owner; do not disable them.

**Rolling back.** Removing the requirement first and then rolling back is true of the **binary**: a build older than the
requirement would otherwise run without being counted. It is not true of the **schema**: the `down` of the migration refuses once
any row of the history exists (the history is evidence), so rolling the schema back needs the history read, exported and removed
on purpose by the owner.

### The first activation: the cron triggers (018)

`control-plane schedule-trigger-handover` requires `schedules-v1` of every live execution engine and, in the same transaction, hands the cron triggers over from the legacy cron pass to the schedule pass. It takes `--operator`, `--expect-live N` and `--dry-run`, runs with the owner role, and is idempotent; the procedure, its refusals and the rollback hazard are in [`schedules.md`](schedules.md) section 6.

## 6. What the registry does not protect against

The registry protects a fleet from **mistakes**: an old build, a flag left off, a restart loop, a process that cannot be reached.
It does not protect against a holder of the runtime role (`sagat_app`) who means harm, because the processes it protects are the
holders of that role. What is closed: a process cannot refresh, delete or rewrite another's row (a row is touched only for the
token that registered it, and the runtime role cannot read the hash), and cannot record a requirement. What is **not**
closed (G2000):

- it can register rows of its own, up to 1,000 live ones, so that new processes find the table full: they serve, uncounted and
  loudly, and an activation cannot see them, so the activation's verification is incomplete;
- it can hold the registration lock open (the epoch share lock and the public advisory lock that counts the table, in a
  transaction it does not finish), so that new processes wait for it and then time out at their statement timeout: they stay
  refused until it lets go;
- it can fail the heartbeat of a victim by holding that victim's row lock, which closes the victim's gate after 30 seconds
  (the failure is not a lost connection).

If you suspect it: look for `pg_stat_activity` sessions of `sagat_app` that are `idle in transaction` for long, and for rows of
`serving_instances` with a `binary_version` or `scheduler_owner` nobody runs; terminate the session and rotate the credentials.

## 7. Upgrades, and why an old binary is invisible

A binary older than the release that introduced the registry does not register, so no activation can see it and no requirement
refuses it (G1011, G1607). The registry therefore ships **in a release before any feature activates through it**:

1. Upgrade the whole fleet, control planes and engines, to a build that registers. Run `control-plane migrate` first.
2. Check `serving_instances_live` equals the replicas you run, that no process of the old build is left, and that the engines
   list what you expect (`SELECT scheduler_owner, capabilities FROM serving_instances`): an engine lists `schedules-v1` only if
   it runs with `SAGAT_SCHEDULES_ENABLED`.
3. Only then activate a feature through the registry.

Rolling back below a recorded requirement is not supported: see "Rolling back" in section 5.

Until the other features move to it, `SAGAT_BATCHES_ENABLED`, `SAGAT_RESET_REAPPLY_ENABLED`, `SAGAT_RESET_CANCEL_SOURCE_ENABLED`
remain per-process settings with the rollout orders of their own runbooks; the registry does not check them (G1999).
`SAGAT_SCHEDULES_ENABLED` is the exception: it decides whether an engine lists `schedules-v1`.

## 8. Symptom table

| Symptom | Likely cause | What to do |
|---|---|---|
| Every replica of a kind answers `503 BINARY_CAPABILITY_MISSING` | A requirement for the kind was recorded and the build you run lacks it, or its configuration does not run the pass | Section 2; or remove the requirement (section 5) |
| One replica answers `503 BINARY_CAPABILITY_MISSING`, the others serve | It is an old build that was silent at the activation and came back, or an engine started without the pass | Upgrade it, set the flag, or stop it |
| An engine is refused after a restart that changed its flags | It no longer runs the pass that provides a required capability | Set the flag again, or remove the requirement if you mean to roll the feature back |
| An activation is refused naming instance ids | Those processes are live and lack the capability (an engine without `SAGAT_SCHEDULES_ENABLED` is one of them) | Upgrade, set the flag, or stop them; query `serving_instances` for their version and capabilities |
| `serving_instances_live` is lower than the replicas you run | A replica is not beating (the log has `could not say this instance is alive`), it was refused, or the table was full when it started | Its log; the database; section 2 |
| `SagatInstanceRegistrySilent` fires | The registry has not answered a process for more than 30 seconds | The database, and the lock waits on `serving_instances` rows; a lost connection leaves the process serving, any other failure closes its gate |
| A row stays for a day after a process died | The process did not leave cleanly; the row is only bookkeeping and stops counting after 30 seconds | None. It is deleted by the day rule, or gives way when the table is full |
| `REGISTRY_UNREACHABLE`, the process is alive and not ready | The migration has not run, or the database is down | `control-plane migrate`; the process registers at its next try (within a second) |
