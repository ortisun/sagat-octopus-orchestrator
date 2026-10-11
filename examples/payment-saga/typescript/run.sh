#!/usr/bin/env bash
# Serves the activities of the payment saga, starts a run of it and waits for it. Run prepare.sh first.
# Needs SAGAT_SERVER, SAGAT_GRPC, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec node dist/main.js "$@"
