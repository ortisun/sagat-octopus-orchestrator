#!/usr/bin/env bash
# Builds the SDK of this checkout, installs the example against it and compiles the example. Idempotent.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
sdk=../../../sdks/typescript

# --ignore-scripts: no dependency runs code of its own at install time (protobufjs has an install script).
npm_install() { npm ci --ignore-scripts --no-audit --no-fund --loglevel=error >&2; }

if [ ! -d "$sdk/node_modules" ] || [ "$sdk/package-lock.json" -nt "$sdk/node_modules/.package-lock.json" ]; then
  (cd "$sdk" && npm_install)
fi
(cd "$sdk" && npx tsc -p tsconfig.json >&2)

npm_install
npx tsc
