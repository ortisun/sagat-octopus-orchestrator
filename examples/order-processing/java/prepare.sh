#!/usr/bin/env bash
# Installs the unpublished Java SDK from this checkout, then builds the standalone example.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
example="$(pwd)"
sdk="$(cd ../../../sdks/java && pwd)"
(cd "$sdk" && ./mvnw -q -B -DskipTests -pl sdk -am install >&2)
"$sdk/mvnw" -q -B --strict-checksums -f "$example/pom.xml" package >&2
test -s target/classpath.txt
