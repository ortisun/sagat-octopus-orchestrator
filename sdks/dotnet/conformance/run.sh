#!/usr/bin/env bash
# Entry point the conformance harness calls: `run.sh build <scenario>` and
# `run.sh run <scenario> <config> <out>`. See tests/contract/sdk-conformance/README.md.
#
# The runner runs on net10.0. SAGAT_DOTNET_TFM=net8.0 runs the net8.0 build of the SDK instead, with the `dotnet` named by
# SAGAT_DOTNET (an install that has the .NET 8 runtime; this machine's own has only 10).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
exec "${SAGAT_DOTNET:-dotnet}" "$here/.conformance/bin/${SAGAT_DOTNET_TFM:-net10.0}/SagatOctopusOrchestrator.Sdk.Conformance.dll" "$@"
