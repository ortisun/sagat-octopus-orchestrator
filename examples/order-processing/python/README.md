# Order processing in Python

This example serves four activities and starts one of the canonical order workflows with the
[Python SDK](../../../sdks/python/README.md). The worker and client run in one process.

## Set up

Start the local stack as described in the [quickstart](../../../docs/quickstarts/README.md), install
the `sagat` CLI, and use Python 3.10 or newer. From this directory, set the connection
settings, publish the workflows and allow the development identity to serve their activities:

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1
export SAGAT_GRPC=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat workflow publish ../workflows/routing.workflow.json
sagat workflow publish ../workflows/fulfillment.workflow.json
sagat workflow publish ../workflows/reminder.workflow.json
sagat worker bind order.validate order.reserve order.quote order.notify

./prepare.sh
./run.sh
```

`prepare.sh` creates `.venv`, installs the SDK dependencies from their hash-checked lockfile, and
installs the SDK from this checkout. It is safe to run again.

## Scenarios

`SAGAT_SCENARIO` defaults to `routing` and accepts these values:

```sh
SAGAT_SCENARIO=routing ./run.sh
SAGAT_SCENARIO=fulfillment ./run.sh
SAGAT_SCENARIO=reminder ./run.sh
SAGAT_SCENARIO=routing SAGAT_REJECT_ORDER=1 ./run.sh
SAGAT_SCENARIO=reminder SAGAT_FAIL_NOTIFY=1 ./run.sh
```

The rejection input has amount `-1`; other runs use `100`. `SAGAT_FAIL_NOTIFY=1` makes
`order.notify` fail permanently with the typed code `NOTIFICATION_REJECTED`. The program prints
only fixed activity names while handlers run, followed by:

```text
scenario routing run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

It exits zero only for `SUCCEEDED`, so the injected notification failure exits 1 after printing
the terminal `FAILED` status. A missing required setting or unknown scenario exits 2.

## External effects and troubleshooting

The reserve and notify handlers are simulations and call no provider. A real external effect is
at least once: send `ctx.idempotency_key` to a provider that deduplicates by that key. If an outcome
is unknown, look it up at the provider before retrying and reconcile the workflow from that result.
Do not assume exactly-once delivery.

An activity can read earlier outputs from optional `ctx.steps` keys: routing can observe `validate`
and fulfillment can observe `reserve` and `quote`. Check each key before consuming it; the reminder
does not require the timer to produce an output.

SDK errors print their stable code, trace id, and remediation. Use the trace id with platform logs;
inspect the run with `sagat workflow inspect <run-id>`. The example never prints the bearer token,
activity input, or step output. If waiting reaches 60 seconds, verify the four activity bindings,
workflow publication, namespace, and gRPC address.
