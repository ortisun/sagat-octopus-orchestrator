#!/usr/bin/env bash
# Copies what the SDKs share with the repository: the worker protocol and the error catalog. `dotnet test`
# has a test that fails when a copy is behind (SharedContractsTests).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"

mkdir -p "$here/proto"
cp "$root/specs/001-platform-foundation/contracts/worker.proto" "$here/proto/worker.proto"
cp "$root/sdks/error-catalog.json" "$here/src/SagatOctopusOrchestrator.Sdk/error-catalog.json"
