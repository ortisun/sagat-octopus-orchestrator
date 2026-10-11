#!/usr/bin/env bash
# Installs the example: a virtualenv in .venv with the SDK from this checkout (until it is published).
# Idempotent. Needs Python >= 3.10 on PATH (override with PYTHON=/path/to/python).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
python="${PYTHON:-python3}"
if [ ! -x .venv/bin/python ]; then
  "$python" -m venv .venv
fi
.venv/bin/python -m pip install --disable-pip-version-check --quiet ../../../sdks/python
.venv/bin/python -c "import sagat_octopus_orchestrator"
