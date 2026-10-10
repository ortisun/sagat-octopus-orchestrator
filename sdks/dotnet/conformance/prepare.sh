#!/usr/bin/env bash
# Builds the SDK and its runner for net10.0 and net8.0. Idempotent. Everything it prints goes to stderr: the harness reads stdout of `run.sh build`.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
bash scripts/sync-shared.sh >&2
for tfm in net10.0 net8.0; do
  dotnet build conformance/SagatOctopusOrchestrator.Sdk.Conformance.csproj -c Release -f "$tfm" -o ".conformance/bin/$tfm" --nologo -v quiet >&2
done
