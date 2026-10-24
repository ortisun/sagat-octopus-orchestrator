# Sagat Octopus Orchestrator: Python SDK

`sagat-octopus-orchestrator` 0.1.0 is the Python SDK of the Sagat Octopus Orchestrator, a durable workflow
platform. It is three things in one package and nothing else (`sdks/SPEC.md`):

1. a **client** of the REST API: start, read, list, cancel and signal runs, publish events (`SagatClient`, on
   `httpx.AsyncClient`; `SyncSagatClient` for code with no event loop);
2. a **worker host** of the gRPC worker protocol: register activities, take leases, heartbeat, complete or
   fail, with cancellation and the saga and reconciliation task kinds (`WorkerHost`, on `grpc.aio`);
3. **builders** of the workflow IR: typed, side-effect-free constructors of the document the server validates
   and versions (`workflow`, `flow`).

The SDK is not a workflow runtime. It executes no workflow code, keeps no journal and decides no transition: the
server owns every state and every retry decision.

## Install

The package is not published to PyPI yet. From a checkout of the repository:

```console
$ python3 -m pip install ./sdks/python        # needs Python >= 3.10
```

It depends on `httpx`, `grpcio` and `protobuf`. The generated protobuf code is committed, so no `protoc` is
needed.

## A run: start it, wait for it

```python
import asyncio
from sagat_octopus_orchestrator import ClientConfig, SagatClient, SagatError


async def main() -> None:
    config = ClientConfig(
        api_url="https://api.example.com/v1",  # including /v1
        namespace="payments",  # the tenant is the token's, never configured
        token="eyJ...",  # or a function (sync or async) called before every request
    )
    async with SagatClient(config) as client:
        run = await client.start("order-flow", {"orderId": "o-1", "amount": "19.99"}, correlation_id="o-1")
        try:
            done = await client.wait_for_completion(run.run_id, timeout=120)
        except SagatError as error:
            print(error.code, error.status, error.retryable, error.trace_id, error.remediation)
            raise
        print(done.status, done.raw)  # `raw` keeps the answer as sent, for fields of a newer server


asyncio.run(main())
```

`start` takes a workflow id or name. Without an `idempotency_key` it makes one per call and reuses it for every
retry of that call, so a lost answer never starts a second run. Amounts of money are strings (`"19.99"`): the
SDK never converts a string to a number, and `600000` is never sent as `600000.0`.

For a large run input, use `await client.upload_run_input(one_pass_iterable, ...)`, then pass the returned
`BlobDescriptor` to `await client.start_with_blob(...)`. The upload is streamed once and never retried; its
descriptor is accepted only after source EOF and matching local size/SHA-256. The synchronous client exposes
the same two operations.

No event loop? `SyncSagatClient` has the same methods, blocking:

```python
from sagat_octopus_orchestrator import SyncSagatClient

with SyncSagatClient(config) as client:
    run = client.start("order-flow", {"orderId": "o-1"})
    for item in client.list_all(status="RUNNING"):
        print(item.run_id)
```

The blocking facade also exposes `update_chain`, `rerun_as_new` and
`wait_for_chain_completion(namespace_id, chain_id)`. The chain wait follows `finalOutcome` across
continuations; `wait_for_completion(run_id)` keeps waiting for that exact segment only.

## An activity: register it, serve it

```python
import asyncio
from sagat_octopus_orchestrator import ActivityContext, ActivityFailure, Verdict, WorkerConfig, WorkerHost


async def charge(order: dict, ctx: ActivityContext) -> dict:
    # ctx.idempotency_key is stable across the attempts of the step: send it to the payment provider.
    if order["amount"] == "0.00":
        raise ActivityFailure.permanent("NOTHING_TO_CHARGE", "the amount is zero")
    return {"chargeId": "c-1"}


async def refund(order: dict, ctx: ActivityContext) -> dict:
    # A compensation undoes ctx.target_node; it must be idempotent and tolerate an effect that never happened.
    return {"refunded": True}


async def lookup(order: dict, ctx: ActivityContext) -> Verdict:
    return Verdict.applied({"chargeId": "c-1"})  # or Verdict.not_applied() / Verdict.unknown()


async def main() -> None:
    host = WorkerHost(
        WorkerConfig(
            grpc_url="https://worker.example.com:443",
            namespace="payments",
            token="eyJ...",
            worker_name="payments-1",
            transport="auto",  # default: stream at protocol 1.4+, poll an older server
        )
    )
    host.register_activity("payments.charge", charge)
    host.register_compensation("payments.refund", refund)
    host.register_reconciler("payments.lookup", lookup)
    await host.start()  # returns once registered
    try:
        await asyncio.Event().wait()  # serve until cancelled
    finally:
        await host.stop(timeout=30)  # waits for the tasks in flight, then cancels the rest


asyncio.run(main())
```

`transport` accepts `"auto"`, `"poll"`, or `"stream"`. Auto is the default and uses finite-credit streaming
when registration accepts protocol 1.4 or newer; it keeps polling compatibility with older negotiated servers.

How an activity ends: a returned value is the step's output (JSON; `None` is `null`; over 256 KiB is a
`PAYLOAD_TOO_LARGE` failure). `ActivityFailure.retryable(code, message)`, `.permanent(code, message)` and
`.outcome_unknown(message)` fail it on purpose. Any other exception is reported as `ACTIVITY_ERROR` with only
its type name as the message, never its text, which may hold the data it was handed.

When worker protocol 1.5 is accepted, `ctx.instance` is an immutable typed copy of the lease identity.
Its `instance_path` holds ordered `ExecutionScope` values and `definition_node_id` names the activated
node. It is `None` at protocol 1.4 or older; reports continue to use the original lease identity.
For a collection body, `ctx.item` is the captured JSON item and `ctx.index` is its stable input index;
the index is `None` for root and non-collection work. The handler's ordinary input remains the run input.

### Cancellation

Cancellation is **cooperative**: the host sets the activity's token and does nothing else. It never interrupts a
handler or injects `asyncio.CancelledError` into its awaits. An activity is cancelled when the run is cancelled
(`run-cancelled`), the lease is lost (`lease-lost`) or the host stops before it finishes (`shutdown`); the handler
looks at `ctx.cancellation` (or `ctx.cancel_reason`) and decides:

```python
async def charge(order: dict, ctx: ActivityContext) -> dict:
    done = asyncio.ensure_future(provider.charge(order, key=ctx.idempotency_key))
    stop = asyncio.ensure_future(ctx.cancellation.wait())  # an awaitable on the token
    await asyncio.wait({done, stop}, return_when=asyncio.FIRST_COMPLETED)
    if not done.done():
        raise asyncio.CancelledError  # "I was cancelled and I say so"
    return await done
```

* a handler that ignores the token runs to its end, and what it returned is reported as usual (unless the lease
  was lost: then nothing is reported);
* a handler that **raises its cancellation error** (`asyncio.CancelledError`, or `SagatError(CANCELLED)` from
  `ctx.cancellation.raise_if_cancelled()`) is read by the reason: after `run-cancelled` it is reported as
  `CANCELLED` (not retried); after `lease-lost` nothing is reported; after `shutdown` nothing is reported either
  (the lease lapses and the server leases the step again; a deploy must not fail a step for good);
* `stop(timeout)` waits for the tasks in flight, then cancels the tokens of the ones left (`shutdown`), gives them a
  moment (2 s) to react and be reported, closes the connection and returns. It never forces a handler.

The token has `is_cancelled`, `reason`, `await wait()`, `wait_blocking(timeout)` for a thread and
`raise_if_cancelled()`. A plain `def` handler is run **in a thread** (it does not block the event loop) and sees the
same token. Inside an `async def` handler, `await run_blocking(function, *args)` runs one blocking call in a thread.
The SDK does not fence the lease by its own clock: only a `STALE_ATTEMPT` answer stops a handler's result from being
reported, so external effects stay at-least-once.

## Workflows: build the document

```python
from sagat_octopus_orchestrator import RetryOptions, flow, workflow

document = (
    workflow("order-flow")
    .description("charges and ships")
    .step("validate", "order.validate", retry=RetryOptions(max_attempts=3, strategy="exponential", jitter=True))
    .saga(
        "settlement",
        flow()
        .step("reserve", "limit.reserve", compensate_with="limit.release")
        .step("charge", "payments.charge", compensate_with="payments.refund", reconcile_with="payments.lookup"),
        cancel_policy="compensate_completed",
    )
    .wait_for_event("paid", event_type="payment.confirmed", correlation_expr="input.orderId", timeout_ms=60_000)
    .parallel("fan", "all", [flow().step("email", "notify.email"), flow().step("sms", "notify.sms")])
    .end("done")
    .build()  # the IR as a dict; .to_json() is the canonical text
)
```

`build()` reads every `flow()` when it is called, not when it is joined: a step added to a branch or a saga body
after `parallel(...)` or `saga(...)` is part of the document. `build()` links node *i* to node *i + 1*, writes nested flows after the top flow and raises
`SagatError(VALIDATION_FAILED)` for what the server would refuse and the builder can see (a duplicate id, an empty
saga, a dangling reference, an `irreversible` step with a compensation...). The canonical text (`to_json()`, RFC 8785: keys
sorted at every depth by UTF-16 code units, not code points, so U+1F600 sorts before U+FF5E; numbers as ECMAScript
prints them; no whitespace) is byte-identical across the five SDKs.

## Errors

Every failure is one `SagatError` with `code`, `message` (safe to log: never a payload, a token or a secret),
`status` (HTTP status, or the gRPC status name), `retryable`, `trace_id`, `reason`, `retry_after_ms` and
`remediation`. The catalog (`sdks/error-catalog.json`) is embedded and a test proves the copy is current. The SDK
retries only when repeating a call cannot do harm (`sdks/SPEC.md` section 5); never a 4xx other than 429. An HTTP
redirect is never followed (it could carry the token to another host): a 3xx is a `PROTOCOL` error with the status
kept. Certificate verification is never switched off (a test reads the source for it).

What a hostile or broken server, or a careless caller, cannot make the SDK do (`sdks/SPEC.md` section 10):

* **No credential in any dump.** The token, the TLS private key, the session token and the lease token are held in a
  small `Secret` wrapper: `repr`, `str`, f-strings, `vars()`, `dataclasses.asdict()` and `json.dumps(..., default=repr)`
  of a config, a client or a host show `<redacted>`, and pickling one raises `TypeError`. `config.token.reveal()`
  returns the token; the SDK calls `reveal()` only where it sends the credential.
* **A body is bounded:** a REST body is read up to 32 MiB (decoded, streamed) and then fails as `PROTOCOL`; a wait the
  server asks for (`Retry-After` in seconds, `retry-after-ms`) is capped at 60 s, and so is the `retry_after_ms` field of
  the error; a server `message` is cut to 512
  characters; a `traceId` that is not 32 lowercase hex characters is not repeated.
* **Names:** a run id is a UUID and a name of `''`, `'.'` or `'..'` is refused before anything is sent.
* **A token provider that raises** is repeated before anything is sent, in every call the SDK repeats and in the
  report of a result (a computed result is not lost to a hiccup of the identity provider); once the attempts are used
  up it is one `SagatError` (`UNAUTHENTICATED`, reason `TOKEN_PROVIDER_FAILED`, the exception's class only). The SDK
  knows it was the provider by a private class, never by the reason: a `TOKEN_PROVIDER_FAILED` in a server's answer
  makes nothing repeatable. It never stops the heartbeat or the poll loop.
* **The host** runs no more tasks than it asked for and registers again with a backoff when its session keeps being
  refused. `ctx.secrets()` hands out copies, overwrites its own buffers when the handler returns, raises afterwards
  (also for a call the handler scheduled and did not await), and a first call after the handler returned does not
  call the server. Python cannot wipe an immutable `bytes` or `str` you made from a secret: that copy is yours to drop.
* **TLS:** `TlsOptions(ca=..., cert=..., key=..., server_name=...)`. `server_name` is the name the server's
  certificate is verified against when it is not the URL's host (SNI and verification both use it); it is verified,
  never skipped, and a wrong one fails as `TRANSPORT`. There is no option, and no code, that turns verification off.

## Units and conventions

* Durations in `ClientConfig`, `WorkerConfig`, `RetryPolicy`, `wait_for_completion`, `stop` and `deadline_in`
  are **seconds** (a `float`; `deadline_in` also takes a `timedelta`). Names that end in `_ms` are milliseconds,
  as the IR and the errors have them (`timeout_ms`, `duration_ms`, `retry_after_ms`).
* Instants the SDK writes are RFC 3339 UTC with a `Z`; an aware `datetime` is required (a naive one is refused).
  Instants it reads are aware `datetime`s in UTC.
* Integers beyond 64 bits, `NaN`, infinities, non-string keys and types JSON has no form for (`Decimal`,
  `datetime`, `bytes`...) are refused before anything is sent, never guessed.
  The canonical text of a workflow (`to_json()`) refuses, by **value**, a whole number beyond 2^53 - 1 that
  ECMAScript would print in digits (below 1e21: `9007199254740993`, `9007199254740992.0`, `1e20`) however it was written
  (it must be the same bytes in every SDK): send it as a string. `1e21` and above, and every number with a fraction,
  are numbers and are written as RFC 8785 writes them.
* Logging uses the standard `logging` module (`sagat_octopus_orchestrator` logger, a `NullHandler` by default);
  a log line holds identifiers and codes only. For tracing pass `trace_context=lambda: traceparent_or_None`.

## Versioning

SemVer. Before 1.0 a minor version may change the Python API; the **wire** contract does not move with it: the
REST API is additive within `/v1` and the worker protocol additive within `1.x`. A newer SDK against an older
server runs the worker at the server's protocol version (negotiated at registration); an older SDK against a newer
server ignores the fields it does not know.
This SDK offers worker protocol `1.5`; the package version remains independent. A server that
negotiates `1.4` or older continues to receive root-compatible requests without scoped identity.

## Runtime matrix: declared and tested

`requires-python = ">=3.10"`. What was actually run, all on macOS 26 arm64 (Darwin 25.3):

| Runtime | Unit tests | `mypy --strict` and `ruff` | Conformance suite |
|---|---|---|---|
| CPython 3.14.8 (Homebrew) | 240 passed | passed | not run for this transport change |
| CPython 3.10.21 (uv build) | 226 passed | not run (type-checked against the 3.10 language level from 3.14) | passed, one run, on an earlier revision of the SDK and of the suite |
| CPython 3.11.15, 3.12.12, 3.13.9 (uv builds) | 226 passed each | not run | not run |
| Linux, Windows, other architectures | not run | not run | not run |
| Free-threaded builds, PyPy | not run | not run | not run |

The dependencies are the exact versions of `requirements.lock.txt` (`httpx` 0.28.1, `grpcio` 1.84.0, `protobuf`
7.36.2 and what they need), each file with its hashes (`pip install --require-hashes`; every pin is at least two weeks
old; `pip-audit` found no known vulnerability in either lock on the day they were made). The ranges in `pyproject.toml` (`httpx>=0.28.1`, `grpcio>=1.84.0` because the generated stubs refuse an
older runtime, `protobuf>=7.35.1` because the generated messages do) were not tried with other versions.

### Not covered

* TLS and mTLS are tested against in-process fakes with throw-away `openssl` certificates (trust roots, client
  certificate, an untrusted server), not against the platform's own TLS endpoints.
* Tracing and metrics use the opt-in `opentelemetry` extra and `OpenTelemetryBridge`, which adapts the existing
  `trace_context` and `on_event` hooks. Real in-memory OpenTelemetry exporters cover propagation, task spans and
  metrics. The application still owns providers and exporters; none is configured by this package.
* No benchmark or soak test: nothing here claims a throughput.

## Development

```console
$ python3 -m venv .venv && .venv/bin/pip install --require-hashes -r requirements-dev.lock.txt
$ .venv/bin/python -m pytest                     # unit tests: offline, a fake HTTP server and a fake gRPC server
$ .venv/bin/mypy && .venv/bin/ruff check .       # strict types and lint
$ .venv/bin/python scripts/sync_shared.py        # copy worker.proto and the error catalog, regenerate *_pb2*.py
$ conformance/prepare.sh && conformance/run.sh   # the conformance runner (the harness calls these)
```

The locks are made from `requirements.in` and `requirements-dev.in` with
`uv pip compile <file>.in --generate-hashes --universal --python-version 3.10 --exclude-newer <two weeks ago> -o <file>.lock.txt`.
`scripts/sync_shared.py` copies `worker.proto` and `sdks/error-catalog.json` from the repository and regenerates
the committed protobuf code; the unit tests fail when any of them is behind. The conformance suite
(`tests/contract/sdk-conformance/`) runs from the repository root:
`SAGAT_SDK_LANGS=python cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture`.

## License

MIT. The wheel and source distribution include the license text in `LICENSE`.

## Managed PostgreSQL connections

Use the database builder with a logical connection created in Sagat. See the
[five-language guide](../../docs/guides/managed-database-sdk.md) for typed parameters,
result consumption, permissions and the shared real-service scenario. Credentials
are resolved by the separate managed executor.
