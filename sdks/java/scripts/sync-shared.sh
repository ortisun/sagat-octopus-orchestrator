#!/usr/bin/env bash
# Copies what the SDKs share with the repository: the worker protocol (compiled into the stubs at build
# time) and the error catalog (read at run time). The unit tests fail when a copy is behind
# (SharedContractsTest); run this script, then the build, to bring them up to date.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"

proto_dir="$here/sdk/src/main/proto"
resource_dir="$here/sdk/src/main/resources/dev/sagatoctopusorchestrator/sdk"
mkdir -p "$proto_dir" "$resource_dir"
cp "$root/specs/001-platform-foundation/contracts/worker.proto" "$proto_dir/worker.proto"
cp "$root/sdks/error-catalog.json" "$resource_dir/error-catalog.json"
echo "synced worker.proto and error-catalog.json" >&2
