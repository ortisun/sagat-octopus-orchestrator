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

## Where to go next

- [The SDK README](../../sdks/python/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the five SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
