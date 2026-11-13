# scripts

## Overview

Shared-contract copying and committed Go binding generation. [generate.sh:PROTOC_GEN_GO_VERSION]

## Interface

- `generate.sh --copy-only` copies shared files without generation; normal mode runs protoc with an SDK-local package mapping. [generate.sh:--copy-only] [generate.sh:protoc]

## Invariants

- The proto copy remains the platform source while generated bindings are placed under internal/workerpb using pinned plugin versions. [generate.sh:PROTOC_GEN_GO_VERSION] [generate.sh:PROTOC_GEN_GO_GRPC_VERSION] [generate.sh:workerpb]

## Side Effects

- When invoked, creates directories, copies shared files, may install generators, runs protoc/gofmt, moves output and removes its temporary generated directory. [generate.sh:mkdir] [generate.sh:cp] [generate.sh:go install] [generate.sh:protoc] [generate.sh:gofmt] [generate.sh:rm]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
