#!/usr/bin/env python3
"""Temporary production dormancy gate and explicit lease-producer inventory (G717).

This source-text check catches ordinary new call sites, not arbitrary Rust aliases/macros or
an adversarial call graph. Runtime authority/race tests remain necessary. Remove the dormancy
ban only with coordinator/creator/rollout acceptance; retain the producer inventory review.
"""
from pathlib import Path
import re

LEASE_PRODUCERS = {
    "crates/application/src/worker.rs",
    "crates/application/src/worker/scoped_claims.rs",
}
ACTIVATION_CALL = re.compile(
    r"\.\s*(?:with_child_root_capacity_guard|(?:acquire|vacate|finish)_current_child_root)\s*\("
)
LEASE_GRANT = re.compile(r"\bLeaseGrant\s*\{")


def check_sources(sources):
    errors = []
    for path, source in sorted(sources.items()):
        parts = Path(path).parts
        if not parts or parts[0] not in ("apps", "crates") or "tests" in parts:
            continue
        if ACTIVATION_CALL.search(source):
            errors.append(f"{path}: capacity activation requires coordinator/rollout acceptance")
        # The port's own type declaration is not a producer.
        if (path not in LEASE_PRODUCERS
                and path != "crates/application/src/execution.rs"
                and LEASE_GRANT.search(source)):
            errors.append(f"{path}: new lease producer requires capacity inventory review")
    return errors


def main():
    root = Path(__file__).resolve().parents[1]
    sources = {
        str(path.relative_to(root)): path.read_text()
        for directory in ("apps", "crates")
        for path in (root / directory).rglob("*.rs")
        if "tests" not in path.relative_to(root).parts
    }
    errors = check_sources(sources)
    for error in errors:
        print(error)
    if not errors:
        print("capacity activation/source inventory check passed")
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
