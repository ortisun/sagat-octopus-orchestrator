#!/usr/bin/env bash
# Serves the two activities, starts a run of hello-world and waits for it.
# Needs SAGAT_SERVER, SAGAT_GRPC, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec .venv/bin/python main.py "$@"
