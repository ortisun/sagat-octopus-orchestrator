#!/usr/bin/env bash
# Serves order activities, starts the selected workflow, and waits for it. Run prepare.sh first.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
binary=bin/order-processing
if [ ! -x "$binary" ]; then
  echo "$binary is not built (run ./prepare.sh first)" >&2
  exit 2
fi
exec "$binary" "$@"
