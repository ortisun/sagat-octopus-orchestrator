# ids

## Overview

Internal cryptographic identifiers for idempotency/event keys. [ids.go:UUID]

## Interface

- UUID generates canonical UUIDv4 text for intra-SDK callers. [ids.go:UUID]

## Invariants

- Randomness failure panics rather than substituting a predictable identifier. [ids.go:UUID]

## Side Effects

- Reads operating-system cryptographic randomness. [ids.go:UUID]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
