#!/usr/bin/env bash
# Serves order activities, starts the selected workflow, and waits for it. Run prepare.sh first.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
dll=bin/Release/net10.0/OrderProcessing.dll
if [ ! -f "$dll" ]; then
  echo "$dll is not built (run ./prepare.sh first)" >&2
  exit 2
fi
exec dotnet "$dll" "$@"
