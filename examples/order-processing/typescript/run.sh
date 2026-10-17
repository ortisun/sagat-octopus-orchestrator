#!/usr/bin/env bash
# Serves order activities, starts the selected workflow and waits for it. Run prepare.sh first.
# Needs SAGAT_SERVER, SAGAT_GRPC, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec node dist/main.js "$@"
