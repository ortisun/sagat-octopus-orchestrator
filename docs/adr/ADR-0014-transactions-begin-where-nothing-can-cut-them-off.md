# ADR-0014 — Transactions begin in a task of their own, because sqlx 0.9.0 can be cut off halfway

Status: **Accepted** | Date: 2026-10-10 | Feature: cross-cutting (found by `fault_004`) | Supersedes nothing

## Context

`tests/fault/004-events-webhooks-messaging/dispatch.rs::a_dispatcher_cut_off_at_any_await…` stops a
dispatcher at random microseconds and then expects the event table to be updatable. In most runs on
`develop` it hung for ever (6 of 8 runs; 2 of 4 on the feature-005 branch): the test's own `UPDATE`
waited on a lock held by a pooled connection that was `idle in transaction`.

The cause is in `sqlx-postgres` 0.9.0, not in the dispatcher. `PgTransactionManager::begin` sends
`BEGIN`, waits for the server, and only then adds one to the connection's transaction depth. A
guard (`Rollback`) is meant to roll back if the future is dropped in between, but it calls
`start_rollback`, which does nothing at depth zero. So a future dropped after the server has executed
`BEGIN` and before the depth is raised leaves the server in a transaction that `sqlx` does not know
about. The connection goes back to the pool; the next statements on it, in autocommit as far as the
code is concerned, run inside that transaction: their writes are never committed and the locks they
take are held until the connection is closed.

Measured with a pool of one connection and 3000 `pool.begin()` futures cut off after 15 to 715
microseconds: **205 left the connection in a transaction** (`sqlx_itself_leaves_the_connection_…`).

This is not a test artefact. Anything that cancels a request while a store begins a transaction
(a client that disconnects, a `timeout`, a task that is aborted, a shutdown) can do it, in every
feature that writes. The window is a few tens of microseconds per transaction, which is why nothing
had shown it in production-like use; a test that cuts at random points shows it at once.

## Decision

**Every transaction in `sagat-infra-postgres` begins through `BeginSafely::begin_safely`**, which runs
`pool.begin()` in a task of its own and awaits it. If the caller is dropped, the task still finishes
the `BEGIN`, the transaction it returns is dropped there and `sqlx` rolls it back as designed. The 51
call sites were changed mechanically; there is no other way into a transaction in the crate.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `after_release` on the pool: send `ROLLBACK` when a connection comes back | One more round trip for every query that takes a connection, to cover a window of microseconds. |
| Read the connection's real transaction status | `sqlx` keeps it private. |
| Upgrade `sqlx` | No fixed release was available when this was written (offline, 0.9.0 is the newest known); revisit at each upgrade. |
| Wrap each `begin()` in a timeout | A timeout is the thing that cuts it off. |
| Accept it | A silent loss of writes after a cancelled request is not an acceptable failure mode for a durable workflow engine (AGENTS.md rule 3). |

## Consequences

* A `tokio::spawn` per transaction: microseconds against a database round trip. The pool is cloned
  (an `Arc`) into the task.
* `tests/fault/004-events-webhooks-messaging/cancellation.rs` holds two tests. One proves that no cut-off
  `begin_safely` leaves a transaction open (0 of 3000). The other proves that `sqlx` still does (a canary):
  **when it fails, `sqlx` has been fixed**, and the helper and the canary can both be removed.
* The dispatcher test that hung now passes 8 of 8 runs.
* Only `begin` was measured. A statement cut off mid-flight is resynchronised by the ping the pool makes when the connection is released (read in `sqlx-core`, and the cut-off dispatcher test passes), and a `COMMIT` cut off after it was sent is followed by a `ROLLBACK` that the server answers with a warning; neither has a test of its own here.

## Review criteria

Revisit when `sqlx` is upgraded (the canary will say whether the helper is still needed) or if a
store begins to use a connection it holds for longer than one transaction (a long-lived `PgConnection`
with its own `begin()` would need the same care).
