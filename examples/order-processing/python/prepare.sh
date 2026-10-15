#!/usr/bin/env bash
# Installs a virtualenv with the pinned SDK dependencies and the SDK from this checkout. Idempotent.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
python="${PYTHON:-python3}"
if [ ! -x .venv/bin/python ]; then
  "$python" -m venv .venv
fi
.venv/bin/python -m pip install --disable-pip-version-check --quiet --require-hashes \
  -r ../../../sdks/python/requirements.lock.txt
.venv/bin/python -m pip install --disable-pip-version-check --quiet --no-deps ../../../sdks/python
.venv/bin/python -c "import sagat_octopus_orchestrator"
