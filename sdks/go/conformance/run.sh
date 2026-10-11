#!/usr/bin/env bash
# Entry point the conformance harness calls: `run.sh build <scenario>` and
# `run.sh run <scenario> <config> <out>`. See tests/contract/sdk-conformance/README.md.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ ! -x "$here/bin/runner" ]; then
  echo "bin/runner is missing: run conformance/prepare.sh first" >&2
  exit 1
fi
exec "$here/bin/runner" "$@"
