#!/usr/bin/env bash
set -euo pipefail
: "${SAGAT_RUST_SDK_TARGET_DIR:?Run prepare.sh with an owned external SAGAT_RUST_SDK_TARGET_DIR}"
runner="$SAGAT_RUST_SDK_TARGET_DIR/debug/sagat-rust-conformance"
if [ ! -x "$runner" ]; then
  echo 'Rust conformance runner is missing; run prepare.sh first' >&2
  exit 1
fi
exec "$runner" "$@"
