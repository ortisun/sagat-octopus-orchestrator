#!/usr/bin/env bash
# Builds the conformance runner into sdks/go/bin/runner (git-ignored). Idempotent: running it again
# rebuilds only what changed (the Go build cache decides).
#
# Dependencies: go.mod and go.sum pin them, and the Go module cache (GOMODCACHE, ~/go/pkg/mod by
# default) holds them. There is no vendor/ directory: it would put the whole of gRPC (tens of
# megabytes) into the repository for the sake of one script. The build is tried offline first
# (GOPROXY=off); only when the cache lacks a module, `go mod download` fetches it once, so after the
# first download this script needs no network.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
export CGO_ENABLED=0 GOFLAGS=-mod=readonly
mkdir -p bin
if ! GOPROXY=off go build -o bin/runner ./conformance 2>/dev/null; then
  echo "the module cache lacks a dependency: downloading it once" >&2
  go mod download >&2
  go build -o bin/runner ./conformance >&2
fi
