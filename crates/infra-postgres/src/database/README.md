# crates/infra-postgres/src/database

## Overview

Crate-internal managed-database admission and policy fencing. It resolves immutable connection revisions and authorization without contacting customer databases.

## Interface

- Crate-private `run_actors`, `admit_definition` and `admit_node` consume run information, scoped identifiers and workflow definitions. [admission.rs:run_actors] [admission.rs:admit_definition] [admission.rs:admit_node]
- Crate-private `DatabasePolicy` holds effective limits and connection/concurrency/provider-operation ceilings. `change` and `read` acquire policy locks/read policy; the enclosing database module remains private outside this crate. [policy.rs:DatabasePolicy] [policy.rs:change] [policy.rs:read] [../database.rs:pub(crate) mod policy]

## Invariants

- Every actor needs `DatabaseUse`; ad-hoc/rerun and write operations require their additional permissions. [admission.rs:check_definition]
- Connections must be ACTIVE, requested/pinned revisions must exactly match the resolved immutable revision, actors must be allowed to use the referenced OpenBao secret, and limits must validate after intersection with connection/policy limits. [admission.rs:check_definition]
- Development-plaintext PostgreSQL is allowed only when the locked namespace environment is `dev`. [admission.rs:check_definition]
- Policy reads hold shared tenant/namespace advisory locks and validate quotas against ceilings. Namespace policy changes retain the shared tenant lock. [policy.rs:read] [policy.rs:change]

## Side Effects

- Admission takes secret/catalog, membership, namespace, connection and secret-access locks and queries PostgreSQL; definition admission records outcome counters and latency histograms. [admission.rs:check_definition] [admission.rs:admit_definition]
- Policy helpers acquire transaction-scoped advisory locks and read quota tables; they do not update policy rows themselves. [policy.rs:lock] [policy.rs:change] [policy.rs:read]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-postgres --locked
```

These commands were not run in this documentation-only pass.
