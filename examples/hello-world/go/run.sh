#!/usr/bin/env bash
# Serves the two activities, starts a run of hello-world and waits for it. Run prepare.sh first.
# Needs SAGAT_SERVER, SAGAT_GRPC, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# The built program, not `go run`: `go run` reports every failure of the program as exit 1.
exec ./bin/hello-world "$@"
