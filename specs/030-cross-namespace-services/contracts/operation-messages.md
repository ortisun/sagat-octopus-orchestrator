# Operation messages and identities — internal contract (draft)

This is the contract between the caller side and the handler side inside one deployment
(ADR-0065). It is not a public API: no client sends or reads these rows directly; the REST
listings of [openapi-service-operations.yaml](openapi-service-operations.yaml) show their kind,
state, attempts and deferral reason. **The worker protocol (`worker.proto`) is unchanged**: no
message, field or minor version is added, and SDK workers need no change (D1508).

## Identities

- `operationId = uuid8(H("sagat.service-operation.v1", tenant, callerRunId, path, node, attempt))`,
  where `attempt` is the call attempt (1, incremented only by an operator `RETRY_STEP`, D1513) and `H`
  is SHA-256 over length-delimited fields (the same domain-separated encoding style as the scoped ids
  of ADR-0030 §4), truncated to 122 bits with the UUIDv8 version and variant bits set. `path` is empty
  in the first increment (root-path nodes only).
- `handlerRunId = uuid8(H("sagat.service-handler-run.v1", tenant, operationId))`. Its shard is
  `run_shard(handlerRunId)` (ADR-0020 §1), known before the run exists.
- A rerun of the caller (`RERUN_AS_NEW`) has a new run id and therefore new operation ids; a retried
  delivery or a crash-restarted pass never changes them.
- Uniqueness: `service_operations (tenant_id, id)`, `service_operation_executions (tenant_id,
  operation_id)` and `(tenant_id, handler_run_id)`, `runs (tenant_id, id)` and
  `service_operation_messages (tenant_id, operation_id, kind)`.

## Message kinds

| Kind | Written by (transaction) | Fields beyond the common ones | Consumed by (transaction) | Effect of a duplicate or late copy |
|---|---|---|---|---|
| `START` | caller, CP1 | — (input digest and deadlines are in the caller record, copied into the message at insert) | admission pass, CP2 / CP3 / deferral | A second claim finds `APPLIED`/`REFUSED`/`WITHDRAWN` and does nothing; a second admission is impossible (derived run id PK) |
| `CANCEL` | caller, CP7 | `policy` | cancellation pass, CP8 | Terminal handler run, `CANCEL_REQUESTED` record, or a handler run already purged (missing): no-op `APPLIED` |
| `STARTED` | handler, CP2 | `handlerRunId` | notification pass, CP4 | Record past `SCHEDULED` (started, settled by an earlier `COMPLETED`, cancelled): no-op `APPLIED`; never `DISCARDED` |
| `START_REFUSED` | handler, CP3 | `outcomeCode` | notification pass, CP4 | Record terminal: `DISCARDED` with audit |
| `COMPLETED` | handler, CP5 | `outcomeCode`, `effect`, `resultDigest` (success only), `resultDisclosed` | notification pass, CP6 | Applies from `SCHEDULED`, `STARTED`, `CANCELLING` or `LOOKING_UP`; record terminal: `DISCARDED` with audit, and its code and effect written once to the record's `late_*` columns |

Common fields: `tenant_id`, `id`, `operation_id`, `kind`, `from_namespace_id`, `to_namespace_id`,
`shard`, `state`, `attempts`, `next_attempt_at`, `created_at`, `settled_at`. The `START` row also
carries `input_digest`, `deadline_at`, `start_deadline_at`, `call_depth`, `endpoint_id`,
`contract_revision`, `operation` and the caller trace context (trace id and computed service-call
span id) so that admission needs no caller-side row.

## Processing rules

1. A pass selects candidates of the shards it holds, `state = 'PENDING' AND next_attempt_at <= now()`
   (database clock), oldest first, bounded batch, `FOR UPDATE SKIP LOCKED`, and fences on the shard
   row in the same transaction (ADR-0020 §3). Candidates are shared between tenants by the existing
   weighted fairness of a pass.
2. The receiving transaction locks its own run first when it mutates one (CP4, CP6, CP8: chain → run →
   record → message, slot 3R of ADR-0050 decision 7); admission (CP2/CP3) reads its shard fence and
   locks the `START` row in slot 3 because its run does not exist yet. No transaction locks a run of
   the sending side. The caller's CP7 locks its own `START` row after its run: the one exception to
   decision 7 (D1660), safe because admission never locks an existing run.
3. Applying is idempotent by state: the receiver checks the record and step it would change; if they
   already show the effect, the message is marked `APPLIED` with no other write; if they are terminal
   and the message would change them, it is `DISCARDED` with an audit record (only `COMPLETED` and
   `START_REFUSED`).
4. **No ordering is assumed** between the notifications of one operation (D1511): candidates of a
   pass run concurrently (ADR-0020), a failed apply backs off, and a `STARTED` can go `DEAD`. A
   `COMPLETED` is therefore applicable while the record is still `SCHEDULED` — `START` and `COMPLETED`
   cannot both exist unless the handler run was admitted, whose id is derived — and a `STARTED` that
   arrives afterwards is a no-op. A `START_REFUSED` excludes the other two (the `START` was refused).
5. A transient failure (serialization failure, unavailable database, quota fence timeout) rolls back
   and a separate transaction increments `attempts` and sets `next_attempt_at` with jittered
   backoff. Notifications and `CANCEL` become `DEAD` at 12 attempts. A `START` deferred for room keeps
   `attempts` unchanged and records `deferral_reason` (D1493).
6. After the commit that writes a message, the outbox publishes a `ServiceOperationMessage` hint
   ([asyncapi-service-operations.yaml](asyncapi-service-operations.yaml)); the pass also polls, so a
   lost hint delays and never loses.
7. Settled rows are purged 7 days after `settled_at` (up to 30 by deployment setting); `PENDING` and
   `DEAD` rows are never purged. The store is registered in 020's content-store registry with the
   erasure hook of [../data-model.md](../data-model.md) (C8). `next_attempt_at`, `created_at` and
   `settled_at` are infrastructure time; `deadline_at` and `start_deadline_at` are workflow time (028).
8. The handler run's deadline is the `START` row's `deadline_at` (the caller's effective deadline), not
   a value recomputed at admission.

## Trace context

`START` carries the caller's trace id and the computed span id of the caller step's service-call
span. The handler run's `trace_id` is the caller's; its root span's parent is that span; the
handler run span carries an OTel link to the caller's service-call span and vice versa, both ids
being computed from durable identities (ADR-0017 §2). Sampling stays a function of the trace id, so
both namespaces reach the same decision.
