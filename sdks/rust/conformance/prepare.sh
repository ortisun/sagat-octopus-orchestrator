#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${SAGAT_RUST_SDK_TARGET_DIR:?Set an owned external SAGAT_RUST_SDK_TARGET_DIR, separate from the backend CARGO_TARGET_DIR}"
python3 - "$here" "$SAGAT_RUST_SDK_TARGET_DIR" "${CARGO_TARGET_DIR:-}" <<'PY'
from pathlib import Path
import sys
source=Path(sys.argv[1]).resolve().parents[2]
target=Path(sys.argv[2])
if not target.is_absolute() or target.resolve().is_relative_to(source) or (sys.argv[3] and target.resolve()==Path(sys.argv[3]).resolve()):
    raise SystemExit('Rust SDK target must be absolute, outside the repository and separate from the backend target')
PY
CARGO_TARGET_DIR="$SAGAT_RUST_SDK_TARGET_DIR" cargo build --manifest-path "$here/Cargo.toml" --locked >&2
