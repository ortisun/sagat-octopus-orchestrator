#!/usr/bin/env bash
# Builds the example; the module cache is populated on the first build.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
go build -o bin/order-processing .
