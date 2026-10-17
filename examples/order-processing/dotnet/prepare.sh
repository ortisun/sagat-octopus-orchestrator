#!/usr/bin/env bash
# Builds the example in Release mode and restores the SDK dependencies on the first build.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
dotnet build OrderProcessing.csproj -c Release --nologo -v quiet
