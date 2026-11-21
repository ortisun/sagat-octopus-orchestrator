# safelog

## Overview

Internal panic-containing slog adapter. [safelog.go:New]

## Interface

- New wraps a logger for SDK consumers; the internal handler implements Enabled, Handle, WithAttrs and WithGroup. [safelog.go:New] [safelog.go:Enabled] [safelog.go:Handle] [safelog.go:WithAttrs] [safelog.go:WithGroup]

## Invariants

- Nil loggers discard records; wrapped handler methods recover panics. [safelog.go:New] [safelog.go:handler]

## Side Effects

- Delegates records to the application handler when it succeeds. [safelog.go:Handle]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
