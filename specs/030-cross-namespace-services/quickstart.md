# Quickstart: a call from `orders` to `payments` (proposed demo, not runnable yet)

This is the reproducible demo the Definition of Done asks for, written before the implementation.
Nothing below runs today: the `sagat service*` commands, the routes and the `serviceCall` node are
proposals of this feature (tasks T030-018, T030-054, T030-060). The parts that exist already are the
local stack, `sagat dev token`, `sagat workflow validate|publish|run|inspect` and
`sagat worker dev --mocks` ([docs/quickstarts/README.md](../../docs/quickstarts/README.md)).
T030-060 turns this page into a checked script and records its real output; until then no outcome
below is evidence.

> **Development only.** The local stack has development credentials and an identity provider that
> signs a token for anyone who asks.

## 1. Stack, identity and two namespaces

```sh
cp infra/docker/.env.example infra/docker/.env
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_TOKEN=$(sagat dev token)      # dev-admin of tenant dev
```

The seed creates namespace `payments` (the handler side). Create the caller namespace `orders` in
the seed's workspace with the existing `createNamespace` route
(`POST /workspaces/{workspaceId}/namespaces`, body `{"slug":"orders","environment":"dev"}`), and keep
both ids:

```sh
export PAYMENTS_NS=<id of payments> ORDERS_NS=<id of orders>
```

## 2. Handler side: workflow, contract, endpoint

`payments/charge-card.json` is an ordinary workflow whose `inputSchema` and `outputSchema` equal the
operation's (copy them from `contracts/service.schema.json`, example operation `charge`), with one
activity `payments.charge` and an `end` whose `outputExpr` returns `{receiptId, status, cardLast4,
holder}`. `payments/mocks.yaml` answers `payments.charge` with a receipt for `orderId: "o-1"` and fails
with `CARD_DECLINED` for `orderId: "o-2"`.

```sh
export SAGAT_NAMESPACE=payments
sagat workflow publish payments/charge-card.json
sagat service publish payments/payments.service.json           # proposed; revision 1, digest printed
sagat service-endpoint create payments --service payments \
  --serve 1 --bind charge=charge-card@1 --caller "$ORDERS_NS"    # proposed; endpoint revision 1
sagat worker dev --mocks payments/mocks.yaml --workflow payments/charge-card.json --bind &
```

Expected: the endpoint is `ACTIVE`, `GET /service-endpoints/payments` lists it with binding health
`COMPATIBLE` and one caller namespace. Publishing `payments.service.json` again prints revision 1 and
`created: false`.

## 3. Caller side: workflow with a `serviceCall`

`orders/place-order.json`:

```json
{
  "apiVersion": "sagat-octopus-orchestrator.io/v1",
  "kind": "Workflow",
  "metadata": {"name": "place-order"},
  "entrypoint": "charge",
  "nodes": [
    {"id": "charge", "type": "serviceCall", "endpoint": "payments", "service": "payments",
     "operation": "charge", "contractRevision": 1, "input": "input.payment",
     "inputDisclosure": "declared-service-input-v1",
     "timeoutMs": 600000, "startTimeoutMs": 60000, "next": "done", "onError": "rejected"},
    {"id": "done", "type": "end", "outputExpr": "steps.charge.output.receiptId"},
    {"id": "rejected", "type": "end"}
  ]
}
```

The file also declares an `inputSchema` whose `payment` property is the operation's input schema (same
annotations): `input.payment` is a direct path, so publication compares each operation input field with
its source and would refuse a source that counts as `pii` feeding an `internal` field
(`INPUT_CLASSIFICATION_LOOSENED`, FR-030-021).

```sh
export SAGAT_NAMESPACE=orders
sagat workflow validate orders/place-order.json
sagat workflow publish orders/place-order.json
```

## 4. Happy path

```sh
sagat workflow run place-order \
  --input '{"payment":{"orderId":"o-1","amount":"10.00","currency":"EUR","cardToken":"tok_demo"}}' --wait
sagat service-operation list --direction outbound          # proposed
```

Expected, to be confirmed by T030-060:

- The caller run is `SUCCEEDED`; its journal shows `service_call.scheduled`, `service_call.started`
  and `service_call.finished` (outcome `SUCCEEDED`, a result digest, no value).
- One handler run of `charge-card` in `payments`, `SUCCEEDED`, whose `run.started` names the
  operation and whose journal ends with `service_operation.completed` (`resultDisclosed: true`).
- `sagat workflow inspect <caller run> --values` shows `status`, shows `receiptId` and `cardLast4`
  to the admin (`internal`) and withholds `holder` unless `--reveal-pii` is used with a grant
  (feature 016), once 016 accepts the `serviceCall` carrier (G1493); before that the result is
  structure-only.
- One trace in the collector contains the caller run, the service-call span and the handler run,
  with reciprocal links. Audit records exist on both sides.

## 5. Failure scenarios

1. **Declared failure.** Run with `orderId: "o-2"`: the handler run fails `CARD_DECLINED`; the caller
   step fails `CARD_DECLINED` with effect class `failed` and the run takes `rejected`.
2. **Allow-list removal in flight.** Make `payments.charge` slow in `mocks.yaml` (`delayMs`), start a
   run, then `sagat service-endpoint disallow payments "$ORDERS_NS"` (proposed) before it ends: the
   handler run still succeeds in `payments`; the caller step fails `OPERATION_ACCESS_REVOKED` with effect
   class `applied`, no result value is retained in `orders`, and the step waits in manual review instead
   of taking `rejected` (an effect-possible failure is never routed as "not done"). A new run is refused at admission with
   `OPERATION_NOT_ALLOWED` and no handler run.
3. **Crash and lost hints.** With a run in flight, stop NATS and kill the execution engine
   (`docker compose ... kill execution-engine`), then start both again: the call completes, exactly
   one handler run exists (`sagat service-operation show <operationId>`), and no message is `DEAD`.
4. **Deadline.** Pause the endpoint (`sagat service-endpoint pause payments --reason demo`, proposed)
   and start a run: the start is deferred (`deferralReason: endpoint_paused`) and after 60 s the caller
   step fails `OPERATION_START_TIMED_OUT`; resuming afterwards starts nothing for that call.

## 6. Clean up

```sh
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env down -v
```
