#!/usr/bin/env bash
# Builds the example: nothing else to install, the module cache is filled on the first build.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
go build -o bin/hello-world .
