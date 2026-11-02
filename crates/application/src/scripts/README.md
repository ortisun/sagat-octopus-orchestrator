# crates/application/src/scripts

## Overview

Typed, bounded sandbox-network incident evidence exported by the scripts module.

## Interface

- `IncidentCategory` exposes seven fixed categories, `ALL`, stable `as_str` codes and rejecting `parse`. [incident.rs:IncidentCategory] [incident.rs:IncidentCategory::parse]
- Non-Debug `IncidentTarget` distinguishes production run/fence and test-run targets and exposes `kind`; storage conversion is parent-private. Public doc-hidden `StoredIncidentTarget` carries identifiers and token hashes. [incident.rs:IncidentTarget] [incident.rs:IncidentTarget::kind] [incident.rs:IncidentTarget::for_storage] [incident.rs:StoredIncidentTarget]

## Invariants

- Only the seven fixed category values parse. Lease tokens must be nonempty and at most 128 bytes before storage conversion. [incident.rs:IncidentCategory::ALL] [incident.rs:IncidentCategory::parse] [incident.rs:MAX_LEASE_TOKEN_BYTES] [incident.rs:IncidentTarget::for_storage]
- Stored evidence contains target identifiers, applicable attempt/generation and a token hash, with no raw token, destination URL/host, headers or payload. [incident.rs:StoredIncidentTarget] [incident.rs:IncidentTarget::for_storage]

## Side Effects

- None.

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-application --locked
```

These commands were not run in this documentation-only pass.
