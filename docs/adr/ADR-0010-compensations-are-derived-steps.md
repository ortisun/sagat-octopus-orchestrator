# ADR-0010 — Compensations and reconciliations are derived steps of the definition

Status: **Accepted** | Date: 2026-10-09 | Builds on ADR-0003, ADR-0004 and ADR-0005; diverges from none.

## Context

ADR-0005 decides that a compensation plan is persisted per saga, in reverse topological order, with retry and manual review. Feature 003 had to decide *how a compensation runs*: as something the engine does on the side of the forward work, or as forward work. The same question was open for the lookup that finds out whether an uncertain effect happened (ADR-0004: external effects are at-least-once, so an ambiguous attempt must be looked up before it is repeated). `specs/003-sagas-compensations/plan.md` also proposed a `crates/saga` crate for the coordinator.

## Decision

1. For every activity with `compensateWith` the parsed definition contains a node `<id>~c`, and for every one with `reconcileWith` a node `<id>~r`. They are ordinary activity nodes (lease, fencing, retry policy, timeout, idempotency template, secrets, bindings), flagged by their origin and unreachable from the author's graph. `~` is outside the alphabet an author may use for an id.
2. A saga is a closed region of the graph. Its coordinator is a function of the run state — `advance(definition, state, stop) → next journal event` — in `crates/domain/src/execution/sagas.rs`, not a crate of its own and not a service with state.
3. The compensation plan of each saga is a pure function of the definition, stored with the published version (immutable) for reading and audit; the plan a run follows is decided when its saga stops going forward, from what completed.
4. Operator decisions on a step that waits for a person are journal events, applied by the same reducer, authorised by permission (`run:operate` for another attempt, `saga:override` to skip or resolve) and audited with a justification in the transaction that records them.
5. The circuit breaker that keeps a provider that is down from being asked by every run is an adapter concern, in the persistence layer: per tenant and activity, written with the attempt's outcome and read where the lease is granted.

## Why

- Everything that already makes forward work durable — the lease and its fence, retries, deadlines, crash and duplicate handling, the journal and its replay, projections, the graph, quotas, bindings, secrets, tracing — applies to a compensation without being written twice, and without a second place where a bug in it can be.
- A coordinator that is a function of the state decides the same way whether it is called by a worker's report, by the scheduler, by a cancellation or by a restart, and a run replayed from its journal reaches the same place (checked by replaying every scenario).
- The state needed to decide what to undo is the whole run state; a separate crate would import it and the architecture check would gain an edge for no isolation.

## Trade-offs

- The graph contains nodes the author did not write. They are marked (`kind`, `forward`) and absent from workflows without a saga.
- The reducer grows: the saga rules sit beside the rules for `parallel` and `child`. They are in their own module and have their own test file.
- A compensation is a step, so a *failed* compensation is a failed step of a run in a state the engine did not have before (`MANUAL_REVIEW`, `COMPENSATING`): old clients meet unknown statuses. The OpenAPI document names them and the change is additive.
- It is still not a rollback of effects that were settled (ADR-0005): it asks the provider to undo them, and says plainly when that was not possible (`PARTIAL`).

## Review criteria

Open a new ADR with benchmark data (`tests/performance/reports/sagas-compensations.md`), failure tests, operational cost and compatibility with runs in flight if compensations ever need to run outside the step machinery (for example as a bulk operation over many runs), or if the breaker needs a key finer than an activity name (G55).
