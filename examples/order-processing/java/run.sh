#!/usr/bin/env bash
# Serves order activities, starts the selected workflow, and waits for it. Run prepare.sh first.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
if [ ! -s target/classpath.txt ] || [ ! -d target/classes ]; then
  echo "the example is not built: run ./prepare.sh first" >&2
  exit 2
fi
exec java -cp "target/classes:$(cat target/classpath.txt)" OrderProcessing "$@"
