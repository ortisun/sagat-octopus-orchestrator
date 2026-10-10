#!/usr/bin/env bash
# Builds the SDK and its conformance runner and writes the runner's classpath. Idempotent. Needs a JDK 17+ and
# network access the first time (the Maven wrapper and the dependencies are downloaded once).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
"$here/scripts/sync-shared.sh"
./mvnw -q -B -DskipTests package >&2
test -s conformance/target/classpath.txt
