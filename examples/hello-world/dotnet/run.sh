#!/usr/bin/env bash
# Serves the two activities, starts a run of hello-world and waits for it (prepare.sh builds it first).
# Needs SAGAT_SERVER, SAGAT_GRPC, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
dll=bin/Release/net10.0/HelloWorld.dll
if [ ! -f "$dll" ]; then
  echo "$dll is not built (run ./prepare.sh first)" >&2
  exit 1
fi
exec dotnet "$dll" "$@"
