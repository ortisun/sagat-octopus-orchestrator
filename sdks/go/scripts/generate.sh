#!/usr/bin/env bash
# Copies what the SDKs share with the repository (the worker protocol and the error catalog) and
# regenerates the protobuf/gRPC code in internal/workerpb. The generated code is committed, so a
# consumer of the SDK needs neither protoc nor this script; the unit tests fail when the copies are
# behind the repository (TestSharedFilesAreTheRepositorys).
#
# Needs `protoc` on the PATH. The two plugins are installed into sdks/go/bin (git-ignored) at the
# versions below, which the generated headers record.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$here"

PROTOC_GEN_GO_VERSION=v1.36.11        # google.golang.org/protobuf, the version go.mod pins
PROTOC_GEN_GO_GRPC_VERSION=v1.6.1     # google.golang.org/grpc/cmd/protoc-gen-go-grpc (2026-02-04)

mkdir -p bin proto/sagat_octopus_orchestrator/v1 internal/workerpb
cp "$root/specs/001-platform-foundation/contracts/worker.proto" proto/sagat_octopus_orchestrator/v1/worker.proto
cp "$root/sdks/error-catalog.json" error-catalog.json

if [ "${1:-}" = "--copy-only" ]; then
  exit 0
fi

if [ ! -x bin/protoc-gen-go ] || [ ! -x bin/protoc-gen-go-grpc ]; then
  GOBIN="$here/bin" go install "google.golang.org/protobuf/cmd/protoc-gen-go@$PROTOC_GEN_GO_VERSION"
  GOBIN="$here/bin" go install "google.golang.org/grpc/cmd/protoc-gen-go-grpc@$PROTOC_GEN_GO_GRPC_VERSION"
fi

# The contract's go_package belongs to the platform, so the SDK overrides it with M... and keeps the
# copy of the proto byte-identical to the contract.
pkg="github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
file="sagat_octopus_orchestrator/v1/worker.proto"
PATH="$here/bin:$PATH" protoc \
  --proto_path=proto \
  --go_out=internal/workerpb --go_opt=paths=source_relative --go_opt="M$file=$pkg;workerpb" \
  --go-grpc_out=internal/workerpb --go-grpc_opt=paths=source_relative --go-grpc_opt="M$file=$pkg;workerpb" \
  "$file"
# paths=source_relative keeps the directory of the proto; the SDK wants the files at the package root.
mv internal/workerpb/sagat_octopus_orchestrator/v1/*.go internal/workerpb/
rm -r internal/workerpb/sagat_octopus_orchestrator
gofmt -l internal/workerpb
