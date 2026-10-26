#!/usr/bin/env python3
"""PostToolUse for Edit/Write: format a Rust file the way CI checks it (`cargo fmt --check`)."""
import json
import subprocess
import sys

try:
    path = (json.load(sys.stdin).get("tool_input") or {}).get("file_path") or ""
    if path.endswith(".rs"):
        subprocess.run(["rustfmt", "--edition", "2024", path], capture_output=True, timeout=30)
except Exception:
    pass
sys.exit(0)
