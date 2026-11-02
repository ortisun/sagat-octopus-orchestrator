# _proto

## Overview

Private committed protobuf/gRPC output consumed by the worker adapter. [worker_pb2.py:DESCRIPTOR]

## Interface

- worker_pb2 registers generated messages/enums/descriptors for worker/lease/report/secret/blob/database wire types. worker_pb2_grpc supplies WorkerServiceStub, unimplemented WorkerServiceServicer, add_WorkerServiceServicer_to_server and experimental WorkerService helpers for all schema RPCs. [worker_pb2.py:DESCRIPTOR] [worker_pb2_grpc.py:WorkerServiceStub] [worker_pb2_grpc.py:WorkerServiceServicer] [worker_pb2_grpc.py:add_WorkerServiceServicer_to_server] [worker_pb2_grpc.py:WorkerService]

## Invariants

- Generated modules validate protobuf runtime 7.35.1 and grpcio compatibility >=1.84.0. Default servicer methods return UNIMPLEMENTED; these bindings do not implement server business guarantees. [worker_pb2.py:ValidateProtobufRuntimeVersion] [worker_pb2_grpc.py:GRPC_GENERATED_VERSION] [worker_pb2_grpc.py:WorkerServiceServicer]

## Side Effects

- Import registers descriptors and checks runtime compatibility. Stub/helper use binds channel calls, performs RPCs or registers service handlers. [worker_pb2.py:DESCRIPTOR] [worker_pb2_grpc.py:WorkerServiceStub] [worker_pb2_grpc.py:add_WorkerServiceServicer_to_server]

## Verification

Run from the repository root:

```sh
cd sdks/python && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
