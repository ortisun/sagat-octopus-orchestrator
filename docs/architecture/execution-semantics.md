# Durable execution semantics

## Architectural choice: state machine + checkpointed steps

V1 uses a **declarative Workflow IR interpreter** and step checkpointing. It does not require deterministic replay of the application's arbitrary stack: the SDKs' DSL is compiled to IR, or SDK workers receive deterministic commands with stable IDs and memoized results. In imperative code-first mode, any code outside steps may re-execute: the SDK MUST forbid/warn about external side effects outside activities.

To avoid promising perfect conversion, we support two modes:

1. **Graph-native:** JSON/IR definition in the Studio or declarative SDK, static and versioned, native execution in the engine.
2. **Code-orchestrated (P2):** orchestrator function in an SDK with controlled replay and step commands; requires a registered worker and proof of determinism. Do not assume immediate support for arbitrary control structures.

## Run states

`CREATED → READY → RUNNING → {WAITING, COMPENSATING, SUCCEEDED, FAILED, CANCELLING, CANCELLED, TIMED_OUT}`

Substates: `WAITING_TIMER`, `WAITING_EVENT`, `WAITING_CHILD`, `WAITING_APPROVAL`; `COMPENSATING → COMPENSATED | COMPENSATION_FAILED`.

- SUCCEEDED, FAILED, COMPENSATED, COMPENSATION_FAILED, CANCELLED and TIMED_OUT are terminal **according to the policy**; manual reprocessing opens a new Run or a new compensation segment, never erases history.
- Run timeout may start compensation; the final state depends on the policy, do not assume TIMED_OUT immediately if compensations are active.
- `PAUSED` keeps state/when valid; it is not terminal.

## Step states

`PENDING → READY → LEASED → RUNNING → {SUCCEEDED, WAITING, RETRY_WAIT, FAILED, SKIPPED, CANCELLED}`; `RETRY_WAIT → READY`, `WAITING → READY`.

### Transactional invariants

**Claim:** the scheduler selects READY with `FOR UPDATE SKIP LOCKED`, increments `generation`, writes `LEASED`, `expires_at` and an event in the journal; writes outbox `TaskAssigned` in the same commit.

**Commit:** the CompleteStep API requires `(run_id, step_id, attempt, lease_token, generation, expected_version, idempotency_key)`. The transaction validates token/fencing + non-terminal; persists output/digest, journal, step update and the following tasks/timers/subscriptions, outbox in the same commit. A duplicate response returns the original result; a stale token returns `409 STALE_ATTEMPT`.

**Recovery:** expired leases return to READY, with a new generation and retry policy; pending outbox is republished. The same worker may try to complete with the old token → reject. The external operation may have already occurred; requires idempotency or reconciliation.

**Persistence:** append-only journal with a monotonic per-Run sequence; snapshots are an optimization and can be regenerated from the journal of decisions and retained data.

## Step identity and determinism

- Each `step_key` is stable and unique within the scope of the logical path (loop: index/element key; parallel: branch key). Do not reuse a step_key for different semantics within the same version.
- `step_id = stable_hash(version_id, run_id, path, step_key)` (derived logical identity, hash collision detected by unique + full key stored).
- Pure functions may be recomputed; side effects only inside activities/steps with a confirmed checkpoint.
- Timestamp, random, UUID in orchestrator code must come from APIs registered as deterministic commands or persisted inputs.
- Large inputs/outputs go to the object store, the journal keeps `blob_ref`, digest and metadata.

## Fork/join, loops, children, race

- Parallel: `READY` branch across multiple steps, join `all/any/quorum` with policy for losers; bounded concurrency.
- Loop: bounded `foreach` with explicit iteration/fanout limit; path includes `iteration_key` and dedupe.
- Child workflow: configurable propagation of cancellation, deadline and trace; `child_run_id` recorded before the task is emitted.
- Race between wait and timeout: row lock on the subscription; first commit wins, the other becomes a duplicate/audited. Define event_time vs receive_time: by default the **commit order** wins.

## Timers, cron and timezone

- Scheduler uses the database clock and the local monotonic clock only for internal scheduling; TTLs persisted in UTC.
- Cron with IANA timezone, explicit DST policy (`skip`, `once`, `twice`), jitter and misfire (`skip`, `catch_up_one`, `catch_up_all_bounded`).
- A wait does not keep a process active; a signal performs atomic `subscription WAITING → FIRED` and creates a READY task.

## Payload, semantics and limits

- Data contract JSON Schema, schema per version and `content_type`. Sensitive values with redaction.
- **Proposed** V1 defaults: inline input 256 KiB, inline output 256 KiB, events 256 KiB, history per Run configurable, 10k physical steps per run as an initial guardrail subject to benchmark. Beyond that: blob storage and child workflows.
- TTL and retention configurable; a run waiting for months is viable as long as retention of the supporting data preserves the resume condition.

## Critical transaction pseudocode

```text
BEGIN;
  SELECT run_version, step_status, generation, lease_token FOR UPDATE;
  IF already_committed(idempotency_key): return prior_result;
  ASSERT step is RUNNING and token/generation/version match;
  UPSERT output_blob_ref_and_hash;
  UPDATE step -> SUCCEEDED;
  INSERT journal(step.completed, ...) WITH run_seq++;
  REDUCE graph + INSERT next READY steps / subscriptions / timers;
  INSERT outbox(task_ready / run_completed / notify);
COMMIT;
-- async outbox publisher publishes task references to NATS;
-- replay/repair reconciler heals missing notification.
```

## Failure matrix and outcome

| Failure | Expected state | Handling |
|---|---|---|
| Worker dies before executing | LEASED expires | Requeue with fencing |
| Worker dies after external action, before ack | Unknown result | Idempotency/reconciliation, safe retry |
| Database commits, broker goes down | Transition persists | Outbox republishes |
| Broker delivers duplicate | One valid claim | Deduplicate and fencing |
| Concurrent completes | Only 1 CAS | Other responds 409 or idempotent return |
| Event arrives before the wait | May be lost without correlated inbox | Subscription-first or specific buffer/reconciliation |
| Timeout races with signal | First commit wins | Auditable events |
| S3 storage unavailable | Step does not complete with lost payload | retry upload, hash and referenced metadata |
