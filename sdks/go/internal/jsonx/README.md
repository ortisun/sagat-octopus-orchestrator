# jsonx

## Overview

Internal faithful JSON encoding, raw-object reading and canonicalization. [jsonx.go:Encode] [canonical.go:Canonical]

## Interface

- Internal callers use Encode, Canonical, Object, String, Int, Bool, Time, IsNull, Instant and ParseInstant. [jsonx.go:Encode] [canonical.go:Canonical] [jsonx.go:Object] [jsonx.go:String] [jsonx.go:Int] [jsonx.go:Bool] [jsonx.go:Time] [jsonx.go:IsNull] [jsonx.go:Instant] [jsonx.go:ParseInstant]

## Invariants

- Encoding rejects cycles/excessive depth, invalid UTF-8, non-finite values, invalid raw JSON, lone surrogates and duplicate keys. Canonicalization sorts UTF-16 keys and rejects digit-form whole numbers beyond 2^53-1. [jsonx.go:maxDepth] [jsonx.go:check] [jsonx.go:checkRaw] [jsonx.go:noDuplicateKeys] [canonical.go:lessUnits] [canonical.go:wholeBeyondExact]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
