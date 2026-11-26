# Python quickstart

The hello-world workflow ([`templates/hello-world`](../../templates/hello-world/workflow.json): `greeting.compose`, then
`greeting.deliver`) served and started with the Python SDK, in [one file](../../examples/hello-world/python/main.py). You need Python 3.10 or newer, the
local stack and `sagat`; the [main quickstart](README.md) gets you there.

## Run it

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat new hello-world
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json
```

then, in [`examples/hello-world/python`](../../examples/hello-world/python), follow its [README](../../examples/hello-world/python/README.md). It prints

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal. The same program in each of the five languages makes the same run,
the same journal and the same outputs, byte for byte, which the end-to-end test of the examples checks.

`prepare.sh` makes a virtualenv in `.venv` and installs the SDK from this checkout (`pip install ../../../sdks/python`), because it is not on
PyPI yet; once it is, that line becomes `pip install sagat-octopus-orchestrator`.

## The worker

An activity receives the **input of the run** and, in the context, the **outputs of the steps that ran before it**:
`ctx.steps["compose"]` is the output of the step with that id. It returns its output. It fails on purpose with a retryable or a permanent failure; anything else it
raises is reported as `ACTIVITY_ERROR`, retryable, with only the type of the error, never its text.

```python excerpt=examples/hello-world/python/main.py
async def compose(run_input: Input, ctx: ActivityContext) -> Greeting:
    return {"greeting": f"Hello, {run_input['name']}!"}


async def deliver(run_input: Input, ctx: ActivityContext) -> Delivery:
    # A real delivery is an external effect: send ctx.idempotency_key to the provider, which is the
    # same for every attempt of this step.
    greeting: Greeting = ctx.steps["compose"]
    print(f"delivering {greeting['greeting']}", flush=True)
    return {"delivered": True, "greeting": greeting["greeting"]}
```

```python excerpt=examples/hello-world/python/main.py
host = WorkerHost(
    WorkerConfig(grpc_url=grpc_url, namespace=namespace, token=token, worker_name="hello-world-python")
)
host.register_activity("greeting.compose", compose)
host.register_activity("greeting.deliver", deliver)
await host.start()
```

The platform delivers at least once, so an activity that makes an effect in the world (a payment, an e-mail) sends
`ctx.idempotency_key` to its provider: it is the same for every attempt of the step, so a repeat does not do it twice. The
platform cannot make the provider exactly-once; the key is how the provider can.

## The client

```python excerpt=examples/hello-world/python/main.py
async with SagatClient(ClientConfig(api_url=api_url, namespace=namespace, token=token)) as api:
    started = await api.start("hello-world", {"name": name})
    print(f"run {started.run_id} started", flush=True)
    done = await api.wait_for_completion(started.run_id, timeout=60)
    print(f"run {done.run_id} {done.status}", flush=True)
```

Starting a run takes the name of a workflow of the namespace (or its id). Without an idempotency key the SDK makes one per
call and reuses it for every retry of that call, so a lost answer never starts a second run. The tenant is never
configured: it is the token's.

## Testing

- **A workflow, before your worker exists:** `sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind` serves its
  activities from a file of canned answers and failures, on the real engine. Make `greeting.compose` fail once
  (`sequence: [{fail: {code: X, retryable: true}}, {output: ...}]`) and see the retry; make it `lose` the task and see the lease
  expire.
- **Your activities, against the platform:** the example is the harness. It is what the end-to-end test of this
  repository runs, with the platform it starts, and what you can run against your own stack.
- **Your activities, alone:** A handler is an `async` function of `(input, ctx)`: keep the effect behind a small interface and unit test the logic by awaiting the
function with a stand-in for the context (a `SimpleNamespace` with the fields it reads). The SDK ships no fake context yet.

## Workflow as code

**Not offered yet.** This section shows the API that [design 044](../designs/044-workflow-as-code.md) specifies; the
Python SDK offers it once its runtime, Task 044-15 of that design, lands. None of it runs today. The calls are those of
[`sdks/SPEC.md`](../../sdks/SPEC.md) section 12, in snake case, with durations in the language's idiom; where the design
leaves a name or a type to the runtime's task (registering the function on the host, the replayer's entry point, the
duration type), the code below is a sketch that Task 044-15 settles. An operator also records
the requirements `code-workflows-v1` and, because this workflow declares a signal, `signal-inbox-v1`
([instance-registry runbook](../runbooks/instance-registry.md)).

Instead of a graph, the workflow is an `async` function. It reserves the stock, charges the card, sleeps one day and
sends a reminder unless a `paid` signal arrived meanwhile (journey 1 of the design):

```python
from datetime import timedelta


async def order(ctx, order_input: dict) -> dict:
    paid = False

    def on_paid(_payload) -> None:
        nonlocal paid
        paid = True

    ctx.on_signal("paid", on_paid)
    await ctx.activity("inventory.reserve", {"orderId": order_input["orderId"]})
    await ctx.activity("payments.charge", {"orderId": order_input["orderId"], "amount": order_input["amount"]})
    await ctx.sleep(timedelta(days=1))
    if not paid:
        await ctx.activity("customers.remind", {"orderId": order_input["orderId"]})
    return {"orderId": order_input["orderId"], "paid": paid}
```

Each `ctx` call is a command the platform records and executes; the activities run in an ordinary worker as above. The
function is run again from the start against the run's journal whenever a worker does not hold it in memory, so it must
be deterministic. The SDK runs it on a deterministic event loop (`asyncio.sleep` and `asyncio.wait_for` become durable
timers too) and imports its modules through an importer of its own: `uuid.uuid4()`, `random.random()`, `os.getenv` and
`open` raise inside a workflow, and `ctx.now()`, `ctx.random()` and `ctx.uuid()` take their place. Its precondition, for
the workflow worker only: Python 3.11 or later, a process time zone of UTC and your own fixed `PYTHONHASHSEED` (any value
but `random` and `0`), the same on every workflow worker of the version and kept private: it is a 32-bit value, so a
reader of the version's digest who knows its modules' sources could search it (Gap 044-016); the SDK caps decoded
objects at 10,000 keys, which bounds the harm whatever the seed.

An activity failure the function does not catch fails the workflow task, not the run: the task is retried after 1, 2, 4
and 8 s, and the fifth consecutive failure puts the run under review. To fail the run on purpose, raise the SDK's
`WorkflowFailure` with a code (Requirement 044-009).

**Write the manifest and publish the version.** The manifest names the code digest, computed from the modules the
workflow imports, the Python version and the hash seed:

```sh
PYTHONHASHSEED="$ORDER_WORKFLOWS_HASH_SEED" python -m sagat_octopus_orchestrator.workflows manifest orders.workflows > manifest.json
```

```python
with open("manifest.json") as file:
    manifest = json.load(file)
envelope = {"activities": [{"name": "inventory.reserve"}, {"name": "payments.charge"}, {"name": "customers.remind"}]}
declarations = {
    "signals": [{"name": "paid", "buffered": True, "maxPending": 10,
                 "schema": {"type": "object", "properties": {}, "additionalProperties": False}}],
}
await api.definitions.publish_code(manifest, envelope, declarations)
```

`publish_code` assembles the code document from the manifest, the envelope and the declarations, saves it as the
workflow's draft and publishes it, making it active (Decision 044-238). The envelope lists what the function's commands
may use; a command outside it puts the run under review. The declarations hold the rest of the code document of
Technical Plan 3.1, here the `paid` signal (`sagat workflow publish-code` takes them with `--declarations <file>`).

**Serve it.** The identity of the workflow worker needs a binding that lists the workflow type `order` (`workflowTypes`
on the worker binding; setting it needs `definition:publish` as well as `worker:bind`). Run it with the same
`PYTHONHASHSEED` and `TZ=UTC`:

```python
workflows = WorkerHost(WorkerConfig(grpc_url=grpc_url, namespace=namespace, token=token, worker_name="order-workflows"))
workflows.register_workflow("order", order)  # the registration's name is fixed by Task 044-15
await workflows.start()
```

**Test it with the replayer.** Export the history of a run (`api.runs.export_history(run_id)`, or
`sagat workflow history export --workflow order --last 64 --dir histories/` once the command ships; both need
`history:export`), keep it with your tests, and replay it against the current code in `pytest`:

```python
import asyncio
from sagat_octopus_orchestrator.workflows import Replayer

from orders.workflows import order


def test_the_new_code_replays_the_recorded_order_runs():
    with open("histories/order-1.jsonl") as history:
        verdict = asyncio.run(Replayer.replay(history, workflow_type="order", workflow=order))
    assert verdict.matched  # the replayer's arguments and verdict fields are fixed by Task 044-15
```

Run the test with the same `PYTHONHASHSEED` as the workers. A mismatch names the first command that differs; a history of
an export format other than 1 is refused, and a replay that reaches a value the export redacted stops with
`INCONCLUSIVE_REDACTED` instead of a false mismatch.

## Where to go next

- [The SDK README](../../sdks/python/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the six SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
