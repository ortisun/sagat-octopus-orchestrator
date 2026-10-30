# workerpb

## Overview

Generated protobuf codecs/reflection and gRPC bindings, internal to the SDK module. [worker.pb.go:File_sagat_octopus_orchestrator_v1_worker_proto] [worker_grpc.pb.go:WorkerServiceClient]

## Interface

- Generated enums/messages/getters/oneofs describe worker, scoped lease, blob and database wire values. [worker.pb.go:DatabaseOperationKind] [worker.pb.go:DatabaseTlsMode] [worker.pb.go:TaskLease] [worker.pb.go:WriteBlobRequest] [worker.pb.go:ReadBlobResponse] [worker.pb.go:DatabaseValue] [worker.pb.go:ResolvedDatabase]
- The package exposes generated method names, NewWorkerServiceClient, WorkerServiceServer, registration, unimplemented defaults, dispatch handlers and the service descriptor. [worker_grpc.pb.go:NewWorkerServiceClient] [worker_grpc.pb.go:WorkerServiceServer] [worker_grpc.pb.go:RegisterWorkerServiceServer] [worker_grpc.pb.go:UnimplementedWorkerServiceServer] [worker_grpc.pb.go:WorkerService_ServiceDesc]

## Invariants

- Generated code checks compatible protobuf/gRPC runtime versions and preserves unknown fields. Server interfaces require forward-compatible embedding unless UnsafeWorkerServiceServer is selected; codecs do not enforce business authorization. [worker.pb.go:EnforceVersion] [worker.pb.go:unknownFields] [worker_grpc.pb.go:SupportPackageIsVersion9] [worker_grpc.pb.go:UnsafeWorkerServiceServer]

## Side Effects

- Client methods invoke unary/stream RPCs and registration attaches handlers to the supplied gRPC registrar. [worker_grpc.pb.go:workerServiceClient] [worker_grpc.pb.go:RegisterWorkerServiceServer]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
