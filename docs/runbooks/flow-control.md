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
   Wait for them, or cancel them through the API if the owner of each run agrees. **Rate-limit rows do not drain in this
   build:** a grant's lease instant is not recorded yet, so a granted slot counts for ever and its window row stays
   (G022-003, T022-074); a rotation with rate-limit rows ends only by the force of section 3. Runs listed only with
   `grants` are such rows, and cancelling them changes nothing.
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
  and waiters drain.**
- Order is kept within each row, not across the two: a new decision can be granted on the new row while older waiters
  still wait on the old one.
- The processes no longer need the retired key; remove it from the rings at the next restart. Digests computed with the
  leaked key stay in the old rows until those rows go (for rate-limit rows, not before T022-074).

Every force is recorded: `SELECT * FROM flow_control_key_forced_retirements;` (version, rows left, your name, the database
role of the session, the reason, the time). The table is append-only, and rolling back its migration is refused while it
has a row. The record is only as good as the roles: run the commands from your own login (not a shared owner login), and
never change `flow_control_key_versions` by hand — a direct `UPDATE` retires a version with no record (G022-006).

## 4. The live-keys ceiling is not exact under concurrency

The namespace's `concurrency_live_keys` ceiling refuses a step whose key would be a new semaphore beyond it
(`FLOW_CONTROL_LIMIT`, through the step's failure route, no attempt spent). Each deciding transaction counts the
namespace's committed keys under the versions in force before it creates its own, without a namespace lock
(OD-2026-11-19-4). So:

- transactions that create keys at the same moment can together pass the ceiling by at most the new keys the others
  create: at most their number minus one when each creates one key; a transaction that releases several gated steps
  with distinct new keys in one commit can add more (O1173);
- after an emergency rotation, the rows the force left are not counted: the ceiling is passed by at most those rows,
  until they drain, and a key filed again is not refused for the row it replaces.

While the count is above the ceiling, a step whose key is new is refused; keys that exist are decided as before.
