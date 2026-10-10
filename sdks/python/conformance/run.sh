#!/usr/bin/env bash
# Entry point the conformance harness calls: `run.sh build <scenario>` and
# `run.sh run <scenario> <config> <out>`. See tests/contract/sdk-conformance/README.md.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
venv="${SAGAT_SDK_PYTHON_VENV:-$here/.venv}"
PYTHONPATH="$here${PYTHONPATH:+:$PYTHONPATH}" exec "$venv/bin/python" -m conformance.runner "$@"
