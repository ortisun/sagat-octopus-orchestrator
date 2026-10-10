#!/usr/bin/env bash
# Entry point the conformance harness calls: `run.sh build <scenario>` and
# `run.sh run <scenario> <config> <out>`. See tests/contract/sdk-conformance/README.md.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec node "$here/dist/conformance/runner.js" "$@"
