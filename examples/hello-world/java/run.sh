#!/usr/bin/env bash
# Serves the two activities, starts a run of hello-world and waits for it. Run ./prepare.sh first.
# Needs SAGAT_SERVER, SAGAT_GRPC, SAGAT_NAMESPACE and SAGAT_TOKEN (see README.md).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
if [ ! -s target/classpath.txt ] || [ ! -d target/classes ]; then
  echo "the example is not built: run ./prepare.sh first" >&2
  exit 2
fi
exec java -cp "target/classes:$(cat target/classpath.txt)" HelloWorld "$@"
