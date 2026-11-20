# bound

## Overview

Internal trust-boundary bounds and sanitizers. [bound.go:MaxServerWait]

## Interface

- Intra-SDK API: MaxServerWait, MaxMessage, MaxReason, Wait, Text and TraceID. [bound.go:MaxServerWait] [bound.go:MaxMessage] [bound.go:MaxReason] [bound.go:Wait] [bound.go:Text] [bound.go:TraceID]

## Invariants

- Waits cap at 60 seconds, text is valid UTF-8 and rune-bounded, and trace IDs require 32 hexadecimal characters normalized lowercase. [bound.go:Wait] [bound.go:Text] [bound.go:TraceID]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
