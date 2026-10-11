#!/usr/bin/env bash
# Builds the example (Release) and the SDK it points at: the NuGet packages are restored on the first build.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
dotnet build HelloWorld.csproj -c Release --nologo -v quiet
