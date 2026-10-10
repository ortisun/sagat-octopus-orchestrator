#!/usr/bin/env bash
# Installs the dependencies into sdks/python/.venv at the pinned versions and checks the SDK imports.
# Idempotent. Needs Python >= 3.10 on PATH (override with PYTHON=/path/to/python); a virtualenv elsewhere
# can be named with SAGAT_SDK_PYTHON_VENV (then it is created and filled the same way).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
venv="${SAGAT_SDK_PYTHON_VENV:-$here/.venv}"
python="${PYTHON:-python3}"
if [ ! -x "$venv/bin/python" ]; then
  "$python" -m venv "$venv" >&2
fi
if [ ! -f "$venv/.runtime-installed" ] || [ requirements.lock.txt -nt "$venv/.runtime-installed" ]; then
  "$venv/bin/python" -m pip install --disable-pip-version-check --quiet -r requirements.lock.txt >&2
  touch "$venv/.runtime-installed"
fi
"$venv/bin/python" -c "import sagat_octopus_orchestrator" >&2
