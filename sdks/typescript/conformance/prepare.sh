#!/usr/bin/env bash
# Installs the dependencies and builds the SDK and its runner. Idempotent.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
if [ ! -d node_modules ] || [ package-lock.json -nt node_modules/.package-lock.json ]; then
  npm ci --no-audit --no-fund --loglevel=error >&2
fi
node scripts/sync-shared.mjs
npx tsc -p tsconfig.json >&2
