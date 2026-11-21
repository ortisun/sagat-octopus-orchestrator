# tokenfail

## Overview

Internal marker for local token-provider failures before request I/O. [tokenfail.go:Failure]

## Interface

- Failure implements Error and Unwrap; Is recognizes this internal marker. [tokenfail.go:Failure] [tokenfail.go:Error] [tokenfail.go:Unwrap] [tokenfail.go:Is]

## Invariants

- The safe marker text excludes provider error text while Unwrap preserves non-panic causes for inspection. [tokenfail.go:Error] [tokenfail.go:Unwrap]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
