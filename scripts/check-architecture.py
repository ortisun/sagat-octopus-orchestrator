#!/usr/bin/env python3
"""Executable architecture rules (AGENTS.md rules 3 and 5, tasks T001-032).

Reads `cargo metadata` and fails when:
  * the domain crate depends on a forbidden infrastructure/format crate, or on a
    non-allowlisted crate at all;
  * a layer depends on a layer it must not know about (application -> adapters, ...);
  * the workspace dependency graph has a cycle.

Usage: python3 scripts/check-architecture.py [--metadata FILE]
  --metadata FILE  read a saved `cargo metadata --format-version 1` JSON instead of
                   running cargo (used by the negative tests).
"""
import json
import subprocess
import sys

# Crates the domain may use (normal dependencies only; dev-dependencies are not checked).
DOMAIN_ALLOWED = {"thiserror", "time", "uuid"}
# Never allowed in the domain, even if someone adds them to DOMAIN_ALLOWED by mistake.
DOMAIN_FORBIDDEN = {
    "axum", "hyper", "tower", "tower-http", "reqwest", "tonic", "prost",
    "sqlx", "tokio-postgres", "postgres", "diesel", "sea-orm",
    "async-nats", "nats", "rdkafka", "aws-sdk-sqs", "kube", "k8s-openapi",
    "serde_json", "tokio", "tracing-opentelemetry", "opentelemetry",
}

# Crates that are application services (they orchestrate ports) rather than adapters or utilities.
APPLICATION_CRATES = {"scheduler", "saga"}

# Layer of each workspace package, derived from its directory under the repo root.
def layer_of(manifest_path: str, root: str) -> str:
    rel = manifest_path[len(root):].lstrip("/") if manifest_path.startswith(root) else manifest_path
    parts = rel.split("/")
    if parts[0] == "apps":
        return "app"
    if parts[0] == "crates":
        name = parts[1]
        if name == "domain":
            return "domain"
        if name == "application":
            return "application"
        if name.startswith("infra-"):
            return "adapter"
        if name.startswith("api-"):
            return "adapter"
        if name in APPLICATION_CRATES:
            return "application"  # services built on the ports: they know the domain and the ports only
        return "support"  # telemetry, ... (no extra restriction beyond acyclicity)
    return "other"

# layer -> layers it may depend on
ALLOWED_LAYER_DEPS = {
    "domain": {"domain"},
    "application": {"domain", "application", "support"},
    "support": {"domain", "support"},
    "adapter": {"domain", "application", "support", "adapter"},
    "app": {"domain", "application", "support", "adapter", "app"},
    "other": {"domain", "application", "support", "adapter", "app", "other"},
}


def load(metadata_file):
    if metadata_file:
        with open(metadata_file) as f:
            return json.load(f)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def main(argv):
    metadata_file = argv[argv.index("--metadata") + 1] if "--metadata" in argv else None
    md = load(metadata_file)
    root = md["workspace_root"]
    ws_ids = set(md["workspace_members"])
    pkgs = {p["id"]: p for p in md["packages"] if p["id"] in ws_ids}
    by_name = {p["name"]: p for p in pkgs.values()}
    errors = []

    graph = {}
    for p in pkgs.values():
        deps = [d for d in p["dependencies"] if d.get("kind") in (None, "normal")]
        graph[p["name"]] = sorted({d["name"] for d in deps})

    for name, deps in graph.items():
        layer = layer_of(by_name[name]["manifest_path"], root)
        if layer == "domain":
            for d in deps:
                if d in DOMAIN_FORBIDDEN:
                    errors.append(f"domain crate '{name}' must not depend on '{d}' (forbidden infrastructure/format crate)")
                elif d not in DOMAIN_ALLOWED:
                    errors.append(f"domain crate '{name}' depends on '{d}', which is not in the domain allowlist {sorted(DOMAIN_ALLOWED)}")
        for d in deps:
            if d in by_name:
                dl = layer_of(by_name[d]["manifest_path"], root)
                if dl not in ALLOWED_LAYER_DEPS[layer]:
                    errors.append(f"layer violation: '{name}' ({layer}) must not depend on '{d}' ({dl})")

    # Cycle detection over workspace members (DFS, three colours).
    WHITE, GREY, BLACK = 0, 1, 2
    colour = {n: WHITE for n in graph}
    stack = []

    def visit(n):
        colour[n] = GREY
        stack.append(n)
        for d in graph[n]:
            if d not in graph:
                continue
            if colour[d] == GREY:
                cycle = stack[stack.index(d):] + [d]
                errors.append("dependency cycle: " + " -> ".join(cycle))
            elif colour[d] == WHITE:
                visit(d)
        stack.pop()
        colour[n] = BLACK

    for n in graph:
        if colour[n] == WHITE:
            visit(n)

    if errors:
        print("architecture check FAILED:")
        for e in errors:
            print(f"  - {e}")
        return 1
    print(f"architecture check passed ({len(graph)} workspace crates, layers respected, no cycles)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
