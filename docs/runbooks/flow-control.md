# Runbook: flow control (feature 022) — the digest key and the live-keys ceiling

For operators of a deployment that turns on concurrency keys or rate limits. This first part covers the two things the
owner decided on 2026-11-19: how the digest key of the flow-control keys is rotated, including in an emergency
(OD-2026-11-19-2), and how far the namespace's live-keys ceiling can be passed (OD-2026-11-19-4). The alerts, redrive and
stuck queues are added with T022-061.

Design: [ADR-0093](../adr/ADR-0093-the-flow-control-digest-key-is-rotated-per-key-with-live-rows.md) (decisions 1 to 5),
ADR-0056 decision 7 (its last sentence superseded), decisions D022-042, D022-043, D022-046 and D022-047, gaps G1190,
G1193, G1196 and G022-002 in [`specs/022-flow-control-step-policies/research.md`](../../specs/022-flow-control-step-policies/research.md).

## 1. The digest key

A concurrency or rate-limit key value is never stored: the database keeps an HMAC digest of it under a deployment key, in
one row per key (`concurrency_semaphores`, `rate_limit_windows`). Every process that decides a gate (control plane,
execution engine, scheduler replica) holds the key ring, and `flow_control_key_versions` says which version is in force:

| State | Meaning |
|---|---|
| `ACTIVE` | new key rows are filed under it; one at most |
| `RETIRING` | rows filed under it still decide their keys; a key that has such a row keeps using it; one at most |
| `RETIRED` | no decision digests under it again, and it never becomes active again; rows a force left under it drain on their own |

Only the operator (not the runtime role `sagat_app`) can run the commands below.

### 1.1 Giving the processes the key ring (D022-057)

The ring is a file, never an environment value: `SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE` names it, and it holds
`version:hex[,version:hex...]`, each key 32 random bytes in hexadecimal (at most 16 KiB). Make one version:

```sh
umask 077 && printf '1:%s\n' "$(openssl rand -hex 32)" > flow-control.keys
# the images run as uid and gid 10001 (infra/docker/Dockerfile; the chart's fsGroup): on a Linux host let that group read it
sudo chgrp 10001 flow-control.keys && chmod 0640 flow-control.keys
```

Give the **same** file to every control plane and every execution engine: on Kubernetes a Secret with the key
`flow-control.keys` named by `flowControl.digestKeys.existingSecret` (the chart mounts it read-only under
`/etc/sagat/flow-control/`); with the local stack the overlay `infra/docker/compose.flow-control.yaml` mounts
`infra/docker/flow-control.keys`. A process logs and prints only the versions of its ring.

A process refuses to start when:

- `SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED` (or the rate switch) is `true` and no file is named, or the file cannot be
  read, is larger than 16 KiB, is not a ring, or holds no key (the error names the variable, never the content);
- the database has a version in force (active, or retiring during a rotation) that the file lacks.

With the ring set, a process lists `flow-control-concurrency-v1` and `flow-control-rate-v1` in the instance registry (its
switches decide only what it admits, D022-062), and only then can the fleet activation of step 3 of 1.2 (or of section 6)
pass. A process of an older build does not list `flow-control-rate-v1`: the activation of `rate-limits` is refused while one
is live.

### 1.2 Turning concurrency keys on

**Not in production before T022-104** (G022-015): publication does not yet refuse a key over a `secret` field. Until
then a key's label and its whole oldest instants are shown only to callers with the namespace's reveal grant; other
readers get the instants truncated to the minute, which narrows but does not stop linking a key's runs (D022-061).

**Turning the switch off** stops new gated publications and starts; keep the key file mounted so that what was admitted
drains: a process with the switches off that is given the file still lists the gate (so a fleet that requires it keeps
serving), grants queued waiters, releases delayed gated starts and decides gated steps (FR-022-031, D022-062). Without
the file a process does not list the gate: once the activation requires it, the registry refuses that process and it
serves nothing, and its queued waiters can only time out. Every process that holds the file checks it against the
versions in force at start, switched on or off. A child run that a draining run starts is a new start: it is refused
`CAPABILITY_DISABLED` (its node's typed failure is T022-102).

1. Make the ring (1.1) and give it to every control plane and engine, with `SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED=true`;
   restart them.
2. `SELECT * FROM flow_control_key_activate(1::smallint, '<your name>');` (expect `ACTIVATED`), as the schema owner.
3. `control-plane flow-control-activate concurrency-keys --operator <you>`: refused, naming the instances, while a live
   process does not list the capability.

## 2. An ordinary rotation

The commands take a `smallint` version: write it `2::smallint` (a bare `2` is an `integer`, and PostgreSQL then finds no
such function). Run each command as a script, not in a psql session left open: while it waits for or holds the version
table, every gated decision of the fleet waits behind it. Start with `SET lock_timeout = '5s';` and retry when it
expires.

1. Add the new version's key to the ring of **every** process and restart them. A process without a version that is
   activated refuses every gated decision until it has it (G1193).
2. `SELECT * FROM flow_control_key_activate(<new>::smallint, '<your name>');`
   - `ACTIVATED`, `out_rows` = the key rows the old version still has (it is `RETIRING`), or 0 (it is `RETIRED` at once);
   - `ROTATION_IN_PROGRESS`, `out_rows` = rows of the version still retiring: one rotation at a time;
   - `ALREADY_ACTIVE`, `RETIRED` (a retired or replaced version is never active again).
3. The rotation ends when the retiring version's last row goes. Its rows belong to runs that hold, wait for or were
   granted a key; a run-level permit lives as long as its run, so one long run keeps a rotation open. To see them:
   `SELECT * FROM flow_control_key_version_runs(<old>::smallint);` (tenant, namespace, run, holders, waiters, grants).
   Wait for them, or cancel them through the API if the owner of each run agrees. Rate-limit rows drain by time: a
   run's grant and a leased step's grant are deleted by the housekeeping once their window row's longest period and one
   more minute have passed since the lease (section 6), and the window row goes with its last grant and waiter. Runs
   listed only with `grants` are such rows: cancelling them changes nothing; wait for the purge. A key in continuous use
   keeps its row under the retiring version (a decision reuses the row that exists): when its traffic never pauses for
   its window row's longest period and one minute, the rotation ends only by the force of section 3.
4. Once `SELECT * FROM flow_control_key_versions_in_force();` answers `retiring = NULL`, remove the old key from the
   rings at the next restart. The next activation records the old version `RETIRED`.

## 3. An emergency rotation (a leaked key)

When a version's key must stop being used at once, without waiting for its rows. Each command answers with a row, not
an error, so **read every outcome before `COMMIT`**, and `ROLLBACK` if one is not the one expected.

| The leaked version is | Run, in one transaction |
|---|---|
| `ACTIVE`, nothing `RETIRING` | `flow_control_key_activate(<new>)` (expect `ACTIVATED`), then `flow_control_key_force_retire(<leaked>, …)` (expect `FORCED`) |
| `ACTIVE`, another one `RETIRING` | `flow_control_key_force_retire(<retiring>, …)`, then `flow_control_key_activate(<new>)`, then `flow_control_key_force_retire(<leaked>, …)` — refused `FORCED_VERSION_DRAINING` while the first force's rows remain (see below) |
| `RETIRING` | `flow_control_key_force_retire(<leaked>, …)` |

```sql
SET lock_timeout = '5s';
BEGIN;
SELECT * FROM flow_control_key_activate(<new>::smallint, '<your name>');
SELECT * FROM flow_control_key_force_retire(<leaked>::smallint, '<your name>', '<reason, e.g. incident number>');
-- read both outcomes; COMMIT only if they are ACTIVATED and FORCED, else ROLLBACK
COMMIT;
```

`flow_control_key_force_retire` answers:

- `FORCED`, with the key rows the version still had;
- `ACTIVE`: the active version is never forced, so activate a successor first;
- `FORCED_VERSION_DRAINING`, with the rows an earlier force left that have not drained. Only one forced version drains at
  a time, so a key never has more than two rows. Wait for those runs, or cancel them
  (`flow_control_key_version_runs(<earlier>::smallint)`);
- `ALREADY_RETIRED`, with the rows it still has;
- `UNKNOWN`.

A reason or name that is blank, longer than 1,000 (255) characters as typed, or holds a line break is refused. **Never
put key material or a digest in the reason**: it is kept for ever. The force waits for every decision in flight, like the
activation.

What it costs, and what to tell the tenants:

- From the commit, a key decided again is filed under the active version, so a key **may have two rows** until the old
  one drains.
- The old row keeps its holders, grants and waiters, gains no new waiter, and grants only its own waiters within its own
  limit. **A key's limit can therefore be exceeded, at any moment, by at most what its old row holds at that moment
  (holders for concurrency, counted grants for a rate limit), never more than the limit, until the old row's holders
  and waiters drain.** The excess is read at each moment and can grow back to the limit while the old row grants its
  own waiters (accepted by the owner, OD-2026-11-20-4).
- Order is kept within each row, not across the two: a new decision can be granted on the new row while older waiters
  still wait on the old one.
- The processes no longer need the retired key; remove it from the rings at the next restart. Digests computed with the
  leaked key stay in the old rows until those rows go (for rate-limit rows, not before T022-074).

Every force is recorded: `SELECT * FROM flow_control_key_forced_retirements;` (version, rows left, your name, the database
role of the session, the reason, the time). The table is append-only, and rolling back its migration is refused while it
has a row. The record is only as good as the roles: run the commands from your own login (not a shared owner login), and
never change `flow_control_key_versions` by hand — a direct `UPDATE` retires a version with no record (G022-006).

## 4. The live-keys ceilings are not exact under concurrency

The namespace's `concurrency_live_keys` ceiling refuses a step whose key would be a new semaphore beyond it, and its
`rate_limit_live_keys` ceiling one whose key would be a new rate-limit window beyond it (`FLOW_CONTROL_LIMIT`, through
the step's failure route, no attempt spent; a run's start is refused with nothing created). Each deciding transaction
counts the namespace's committed key rows of that kind under the versions in force before it creates its own, without a
namespace lock (OD-2026-11-19-4; for the rate-limit keys OD-2026-11-20-11). So, for each of the two ceilings:

- transactions that create keys at the same moment can together pass the ceiling by at most the new keys the others
  create: at most their number minus one when each creates one key; a transaction that releases several gated steps
  with distinct new keys in one commit can add more, so the bound is in keys, not transactions: at most the new keys
  the other transactions of the same moment create (accepted by the owner, OD-2026-11-20-3);
- after an emergency rotation, the rows the force left are not counted: the ceiling is passed by at most those rows,
  until they drain, and a key filed again is not refused for the row it replaces.

While the count is above the ceiling, a step whose key is new is refused; keys that exist are decided as before.

A run's start counts under the namespace's admission lock, so two starts never pass a ceiling together; a step's
readiness round takes no such lock. A rate-limit window row stays while a waiter or a grant the purge must keep uses it
(its window row's longest period and one more minute after the grant's lease), so a key used once counts for that long.
When a new rate-limit key would pass the ceiling, the deciding transaction first deletes, in its own namespace, windows
that no waiter and only grants the purge may delete keep (at most 64 grants each, never one another transaction holds),
and decides with the room that frees; a namespace never waits for another tenant's purge backlog, except for a window of
more than 64 grants (G022-018).

## 5. Queued runs (a run's start that waits for its permit)

A workflow with a top-level `concurrency` takes a permit of its key when a run is started; when the key is full the run
is created `WAITING` and queued (`concurrency.queued`), and the scheduler grants the oldest waiter when a holder ends
(`concurrency.acquired`) or fails it `CONCURRENCY_WAIT_TIMEOUT` at its queue deadline. A synchronous start is refused,
with nothing created, when the key's queue is full (`CONCURRENCY_QUEUE_FULL`), the namespace's `queued_runs` is reached or
the key would be new beyond `concurrency_live_keys` (`FLOW_CONTROL_LIMIT`). Each refusal is counted
`concurrency_acquisitions_total{scope="run",outcome="refused_full"}`.

- `queued_runs` is exact: every start holds the namespace's admission lock while it is decided (D022-051).
- A start delay takes its permit or queues at its release, and a reset takes a fresh permit or is refused
  `CONCURRENCY_PERMIT_UNAVAILABLE` (D022-052, D022-053). A workflow with a `forEach`, `while` or `repeatUntil` node
  beside a top-level gate is refused `BINARY_CAPABILITY_MISSING` in this build (G1197). A reset of a run whose start
  declares `rateLimit` takes a fresh slot or is refused `RATE_LIMIT_SLOT_UNAVAILABLE`, rate first (T022-086, D022-069).
- Rolling back migration `20270203000000` is refused while any run, finished or not, keeps `runs.flow_control_gates`:
  archive those runs first.

## 6. Rate limits (T022-072, T022-074, T022-077)

**Not in production before T022-080, T022-086 and T022-104, and gap G022-018** (ADR-0088's fault suite; T022-086: the reset
of a rate-limited run, built and under review, until it is ticked; G022-015; G022-018: the deployment-wide purge scans every
grant a call, in no order, and has no backlog gauge). The consistency check of the rate tables across runs (T022-079) and
the lock-order barriers (T022-081) are done. Finish the rolling upgrade before resetting a rate-limited run: a replica built
before T022-086 cannot read the `restored` copy of a `rate_limit.*` event. Finish the rolling upgrade before setting any `rate_limit_*` limit (an older
replica's quota listing refuses a name it does not know, G1192); rolling back migration `20270206000000` refuses while one
is set.

A process with the ring lists `flow-control-rate-v1` beside the concurrency gate. Turn it on as 1.2, with
`SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED=true` and `control-plane flow-control-activate rate-limits --operator <you>`.

- A run's start that declares `rateLimit` takes a slot of its key when it is admitted, or is buffered `WAITING`
  (`rate_limit.queued`); a run's slot counts from its grant and is never given back: it stops counting after its window.
  A synchronous start is refused `RATE_LIMIT_QUEUE_FULL` when its key's buffer (`flow_control_key_waiters`) or the
  namespace's `queued_runs` is full, rate first for a start that also declares `concurrency`, and `FLOW_CONTROL_LIMIT`
  (`rate_limit_live_keys`) when its key would be a new window beyond the namespace's ceiling.
- A step's slot counts while it is granted and not leased, then for its window from the lease. The scheduler copies the
  lease instant from the journal each tick (`scheduler_actions_total{kind="rate_lease"}`, `outcome` `done` or `failed`);
  a late copy only keeps a slot longer, and a waiter is visited at the latest one copy after its slot frees. With no
  worker, at most `limit` granted steps of a key wait unleased.
- The effective limit is the lowest of the declaration and the namespace's `rate_limit_per_second` (20 at most until the
  published benchmark), `rate_limit_per_minute` (6,000) or `rate_limit_per_hour` (10,000), by the declaration's window.
- The housekeeping deletes a grant once its window row's longest period and one more minute have passed since its lease
  (`scheduler_actions_total{kind="rate_purge"}`), at most 64 keys and 500 grants a key a call, skipping a key another
  transaction holds. A window row counts against `rate_limit_live_keys` until it goes; a decision that needs room
reclaims its own namespace's purgeable windows first (section 4). To see the backlog of a namespace: `SELECT count(*)
FROM rate_limit_grants g JOIN rate_limit_windows w USING (tenant_id, namespace_id, pool, limit_scope, group_name,
key_digest) WHERE g.namespace_id = '<namespace>' AND g.leased_at + make_interval(secs => w.max_period_ms / 1000.0 + 60)
< clock_timestamp();`.
  A definition published later with a longer window on the same group and key (`RATE_LIMIT_MISMATCH` warns at
  publication) does not count the grants already deleted, so in its first window it may be admitted up to that many
  early (G022-017): give definitions that share a group the same window.
