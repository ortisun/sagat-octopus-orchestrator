#!/usr/bin/env bash
# Entry point the conformance harness calls: `run.sh build <scenario>` and
# `run.sh run <scenario> <config> <out>`. See tests/contract/sdk-conformance/README.md.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
list="$here/conformance/target/classpath.txt"
if [ ! -s "$list" ] || [ ! -d "$here/conformance/target/classes" ]; then
  echo "the runner is not built (or is being rebuilt): run conformance/prepare.sh first" >&2
  exit 2
fi
classpath="$here/conformance/target/classes:$(cat "$list")"
# Short-lived JVM: one scenario per launch, so favour start-up over peak speed.
exec java -XX:+UseSerialGC -XX:TieredStopAtLevel=1 -Xshare:auto -Dfile.encoding=UTF-8 \
  -cp "$classpath" dev.sagatoctopusorchestrator.conformance.Runner "$@"
