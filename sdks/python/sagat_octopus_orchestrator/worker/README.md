# worker

## Overview

Worker and activity API over an internal authenticated grpc.aio adapter. [__init__.py:__all__] [_grpc.py:WorkerRpc]

## Interface

- Worker exports include WorkerHost/Config/Transport/Event, ActivityContext/Handler/Failure, Verdict, CancellationToken, CancelReason, TaskKind, Secrets, blob/identity contracts and run_blocking. [__init__.py:__all__]
- WorkerHost registers activities/compensations/reconcilers and exposes protocol_version, start/stop and async context management. ActivityContext exposes lease/instance/item/index identity, idempotency/target keys, outputs, cancellation/deadline/logging, secrets and blob I/O. [host.py:WorkerHost] [activity.py:ActivityContext]
- ActivityFailure and Verdict describe deliberate outcomes; CancellationToken supports async/blocking observation and Secrets returns copied values. Private WorkerRpc supplies the host RPCs but omits the copied schema's additive 1.6 ResolveDatabase. [activity.py:ActivityFailure] [activity.py:Verdict] [activity.py:CancellationToken] [activity.py:Secrets] [_grpc.py:METHODS]

## Invariants

- The host supports 1–1000 inflight slots and 1–256 unique pre-start handlers; task dispatch respects receive credit and the 8 MiB stream input bound. [host.py:WorkerHost] [host.py:MAX_RECEIVE_INPUT_BYTES]
- Cancellation sets a cooperative token reason; stale/lost results are dropped and shutdown cancellation errors are not reported. Completion/failure uses stable keys and bounded safe retries. [host.py:_cancel_activity] [host.py:_conclude] [host.py:_with_retries] [host.py:_complete] [host.py:_fail]
- The host offers 1.5. Task kinds/secrets require minor 2, blobs 3, streams 4 and scoped identity 5. Inline outputs cap at 256 KiB; blob streams validate ordering, chunks, size and SHA-256. [../config.py:PROTOCOL_VERSION] [host.py:_scoped_instance] [host.py:MAX_OUTPUT_BYTES] [host.py:_validate_blob_descriptor] [host.py:_read_blob]
- Secret resolution is lazy and attempt-scoped; returned values are copies and internal buffers are invalidated/wiped when the handler finishes, including late resolutions. [host.py:_secrets_of] [activity.py:_drop]

## Side Effects

- Opens/closes gRPC channels and performs registration, lease reception, heartbeat, secret/blob and result RPCs. Creates asyncio receive/handler/heartbeat tasks and a bounded thread pool for blocking handlers. [_grpc.py:WorkerRpc] [host.py:_start] [host.py:_stop]
- Invokes caller handlers/parsers/providers/loggers and optional telemetry hooks; stop signals handlers and closes receive resources. [host.py:_invoke] [host.py:_emit] [host.py:_stop]

## Verification

Run from the repository root:

```sh
cd sdks/python && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
