#!/usr/bin/env bash
# Builds the example. The SDK is not on a repository yet, so it is installed from this checkout into the local
# Maven repository (~/.m2) first; then the example is compiled and its runtime classpath is written to
# target/classpath.txt. Idempotent. Needs a JDK 17+ and, the first time, network access (the Maven wrapper and
# the dependencies are downloaded once).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
example="$(pwd)"
sdk="$(cd ../../../sdks/java && pwd)"

# The SDK, from its own directory: the wrapper reads .mvn/maven.config (--strict-checksums) from where it is started.
(cd "$sdk" && ./mvnw -q -B -DskipTests -pl sdk -am install >&2)

# The example, with the SDK's wrapper (there is no system mvn) started from the example's directory: the wrapper
# finds its distribution next to itself, but not the SDK's .mvn/maven.config, so the same flag is given here.
"$sdk/mvnw" -q -B --strict-checksums -f "$example/pom.xml" package >&2
test -s target/classpath.txt
