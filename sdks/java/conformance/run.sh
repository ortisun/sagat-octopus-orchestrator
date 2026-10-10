#!/usr/bin/env bash
# Entry point the conformance harness calls: `run.sh build <scenario>` and
# `run.sh run <scenario> <config> <out>`. See tests/contract/sdk-conformance/README.md.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
classpath="$here/conformance/target/classes:$(cat "$here/conformance/target/classpath.txt")"
# Short-lived JVM: one scenario per launch, so favour start-up over peak speed.
exec java -XX:+UseSerialGC -XX:TieredStopAtLevel=1 -Xshare:auto -Dfile.encoding=UTF-8 \
  -cp "$classpath" dev.sagatoctopusorchestrator.conformance.Runner "$@"
