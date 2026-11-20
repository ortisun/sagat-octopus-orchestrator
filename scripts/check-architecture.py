#!/usr/bin/env python3
"""Executable architecture rules (AGENTS.md rules 3 and 5, tasks T001-032).

Reads `cargo metadata` and fails when:
  * the domain crate depends on a forbidden infrastructure/format crate, or on a
    non-allowlisted crate at all;
  * a layer depends on a layer it must not know about (application -> adapters, ...);
  * a package other than the test package depends on the `iceberg` crates, whatever the kind;
  * the lake runner reaches the container engine, directly or through any crate it links (ADR-0079, rule 2);
  * the workspace dependency graph has a cycle.

Usage: python3 scripts/check-architecture.py [--metadata FILE]
  --metadata FILE  read a saved `cargo metadata --format-version 1` JSON instead of
                   running cargo (used by the negative tests).
"""
import json
import subprocess
import sys

# Crates the domain may use (normal dependencies only; dev-dependencies are not checked).
DOMAIN_ALLOWED = {"thiserror", "time", "uuid", "zeroize"}
# Never allowed in the domain, even if someone adds them to DOMAIN_ALLOWED by mistake.
DOMAIN_FORBIDDEN = {
    "axum", "hyper", "tower", "tower-http", "reqwest", "tonic", "prost",
    "sqlx", "tokio-postgres", "postgres", "diesel", "sea-orm",
    "async-nats", "nats", "redis", "rdkafka", "aws-sdk-sqs", "kube", "k8s-openapi",
    "serde_json", "tokio", "tracing-opentelemetry", "opentelemetry",
}

# The `iceberg` crate and its `iceberg-*` siblings may be used by the test package only (any kind).
ICEBERG_CRATE = "iceberg"
ICEBERG_TEST_ONLY_PACKAGES = {"sagat-tests"}

# ADR-0079, rule 2: the lake runner holds no container-engine access. The sandbox runner is the one process that links the engine
# (`bollard`, through the crate that wraps it), and the lake runner asks it for an attempt over a socket instead.
LAKE_RUNNER_PACKAGES = {"sagat-database-runner", "sagat-infra-lake"}
CONTAINER_ENGINE_CRATES = {"bollard", "sagat-infra-sandbox"}

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

    # The Iceberg crates are the independent reader of feature 035 (D035-026): test-only. Writes run in the
    # Python lake runtime (ADR-0071), so no shipped package may link them, whatever the dependency kind.
    for p in pkgs.values():
        if p["name"] in ICEBERG_TEST_ONLY_PACKAGES:
            continue
        for d in sorted({d["name"] for d in p["dependencies"]}):
            if d == ICEBERG_CRATE or d.startswith(ICEBERG_CRATE + "-"):
                errors.append(
                    f"'{p['name']}' must not depend on '{d}': the Iceberg crates are test-only "
                    f"(the independent reader lives in the tests package; writes use the Python lake runtime, ADR-0071)"
                )

    # What the lake runner links, directly or through any crate it links: the way in is named, so the edge to cut can be seen. Normal and
    # build dependencies count (they end up in what ships or in how it is built); a dev dependency does not (nothing that ships links it).
    build_deps = {
        p["name"]: sorted({d["name"] for d in p["dependencies"] if d.get("kind") == "build"})
        for p in pkgs.values()
    }
    if "sagat-application" in graph:
        for missing in sorted(LAKE_RUNNER_PACKAGES - graph.keys()):
            errors.append(
                f"'{missing}' is named by ADR-0079 and the workspace has no such package: the rule would check nothing "
                f"(rename it in LAKE_RUNNER_PACKAGES with the package)"
            )
    for holder in sorted(LAKE_RUNNER_PACKAGES & graph.keys()):
        seen = {holder: [holder]}
        queue = [holder]
        while queue:
            current = queue.pop(0)
            for dep in graph.get(current, []) + (build_deps.get(current, []) if current == holder else []):
                if dep not in seen:
                    seen[dep] = seen[current] + [dep]
                    if dep in graph:
                        queue.append(dep)
        for engine in sorted(CONTAINER_ENGINE_CRATES & seen.keys()):
            errors.append(
                f"'{holder}' must not reach '{engine}' ({' -> '.join(seen[engine])}): the lake runner holds no container-engine access, "
                f"it asks the sandbox runner for an attempt (ADR-0079, rule 2)"
            )

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
