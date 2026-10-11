#!/usr/bin/env bash
# Serves the two activities, starts a run of hello-world and waits for it.
# Needs SAGAT_API_URL, SAGAT_GRPC_URL, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec go run . "$@"
