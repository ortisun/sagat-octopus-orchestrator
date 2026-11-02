# OpenBao capability policies

## Overview

ACL policies consumed by the development and Kubernetes bootstrap scripts.

## Interface

- `management.hcl` grants create/read on `secret/data/sagat/credentials/*`; `runtime.hcl` grants read on that path. Both grant token self lookup, renewal and revocation. [management.hcl:capabilities] [runtime.hcl:capabilities]
- `cleanup.hcl` grants metadata read, data create and version destruction update, together with the same token self operations. [cleanup.hcl:capabilities]

## Invariants

- Cleanup has no credential-value read or metadata-delete grant; its comment requires permanent CAS tombstones on retirement. These files declare grants; OpenBao enforces the installed policies. [cleanup.hcl:capabilities]

## Side Effects

- None; these are policy declarations.

## Verification

Run from the repository root:

```sh
cargo test -p sagat-tests --test integration_database_connections --locked
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../../../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
