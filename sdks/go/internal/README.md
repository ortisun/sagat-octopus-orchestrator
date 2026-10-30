# internal

## Overview

Directory boundary for backoff, bound, ids, jsonx, safelog, tokenfail, workerpb.

## Interface

- [backoff](./backoff/README.md) — component contract.
- [bound](./bound/README.md) — component contract.
- [ids](./ids/README.md) — component contract.
- [jsonx](./jsonx/README.md) — component contract.
- [safelog](./safelog/README.md) — component contract.
- [tokenfail](./tokenfail/README.md) — component contract.
- [workerpb](./workerpb/README.md) — component contract.

## Invariants

- None at this grouping level; child contracts describe their own checks.

## Side Effects

- None at this grouping level; runtime effects belong to the child components.

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
